//! Provisional embedding prototypes, deliberately outside the stable API.
//!
//! Enable `experimental-embedding` to run this investigation. Names, signatures,
//! duplicate registration policy, and provenance collection costs are not settled.

use std::{ffi::OsString, ops::Range, sync::Arc};

use nickel_lang_core::{
    cache::{InputFormat, SourcePath, normalize_path},
    eval::{VirtualMachine, value::NickelValue},
    files::{FileId, Files},
    position::{PosTable, TermPos},
    term::Term,
};

use crate::{Context, Error, Expr};

/// Provisional input formats. Deliberately separate from core's extensible enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SourceFormat {
    Nickel,
    Toml,
}

impl From<SourceFormat> for InputFormat {
    fn from(format: SourceFormat) -> Self {
        match format {
            SourceFormat::Nickel => Self::Nickel,
            SourceFormat::Toml => Self::Toml,
        }
    }
}

/// Provisional opaque identity. Equality includes the originating context.
/// Names are display information, never identity. Synthetic diagnostic sources
/// use a separate identity scope for each extraction.
#[derive(Clone, Debug)]
pub struct SourceId {
    scope: Arc<()>,
    file: FileId,
}

impl PartialEq for SourceId {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.scope, &other.scope) && self.file == other.file
    }
}

impl Eq for SourceId {}

/// An owned source snapshot. Cloning shares the UTF-8 source allocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDocument {
    pub id: SourceId,
    pub name: OsString,
    pub text: Arc<str>,
}

/// A zero-based, half-open UTF-8 byte range in a particular source snapshot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub source: SourceId,
    pub range: Range<usize>,
}

/// A component of a parsed input path. Field names containing dots stay distinct.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PathElement {
    Field(String),
    Index(usize),
}

/// One original parsed value, before evaluation, contracts, or defaults.
/// A missing location does not imply that the value is absent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputLocation {
    pub path: Vec<PathElement>,
    pub location: Option<SourceLocation>,
}

/// Provisional context-bound source handle. It keeps its source snapshot alive,
/// but cannot evaluate without its originating context. Fields are private so
/// callers cannot forge handles or replace the registered source.
#[derive(Clone, Debug)]
pub struct Source {
    document: Arc<SourceDocument>,
    format: SourceFormat,
    provenance: Arc<[InputLocation]>,
}

impl Source {
    pub fn document(&self) -> &SourceDocument {
        &self.document
    }

    pub fn format(&self) -> SourceFormat {
        self.format
    }

    /// Locations in the original syntax only. No evaluated/defaulted provenance
    /// is inferred. For Nickel, this visits literal containers only; it does not
    /// traverse computations or imports. TOML contains only data containers.
    pub fn input_locations(&self) -> &[InputLocation] {
        &self.provenance
    }
}

/// Provisional evaluation choices, matching the existing Context entry points.
#[derive(Clone, Copy, Debug)]
pub enum EvaluationMode {
    Shallow,
    Deep,
    DeepForExport,
}

impl Context {
    /// Register and parse a named immutable input with an explicit format.
    ///
    /// The name is also its filesystem origin. Relative names are resolved at
    /// registration, without resolving symlinks. Diagnostics preserve the supplied
    /// name. Re-registering a normalized path (even with another format) fails.
    /// A syntax error releases the name for retry and retains its diagnostic source.
    /// Named imports still follow Nickel's normal format rules: a TOML source
    /// named `data.ncl` requires `import "data.ncl" as 'Toml`. An inferred Nickel
    /// import does not select a TOML registration solely by matching its name.
    /// Parsing uses the existing term cache; evaluation reuses this cached input.
    /// This prototype eagerly snapshots input provenance to keep it independent
    /// of subsequent transformations; that cost needs separate API review.
    pub fn add_source(
        &mut self,
        name: impl Into<OsString>,
        format: SourceFormat,
        text: impl Into<String>,
    ) -> Result<Source, Error> {
        let name = name.into();
        let normalized = normalize_path(std::path::PathBuf::from(&name))?;
        let sources = &mut self.vm_ctxt.import_resolver.sources;
        if sources.file_paths.values().any(|path| {
            matches!(path, SourcePath::Path(p, _) if normalize_path(p).ok().as_ref() == Some(&normalized))
        }) {
            return Err(invalid_input("source path is already registered"));
        }

        let original_path = SourcePath::Path(name.clone().into(), format.into());
        let id = sources.add_string(original_path.clone(), text.into());
        // SourceCache normally uses the same path for display and lookup. Keep
        // the original display name in Files, and use an absolute origin and
        // lookup key so ordinary imports can find this source without #2362's
        // magic prefix. This is private cache wiring, not a public VFS contract.
        let entry = sources.file_ids.remove(&original_path).unwrap();
        let canonical_path = SourcePath::Path(normalized, format.into());
        sources.file_paths.insert(id, canonical_path.clone());
        sources.file_ids.insert(canonical_path.clone(), entry);

        let parse_result = (|| {
            if format == SourceFormat::Nickel {
                self.vm_ctxt.import_resolver.parse_to_ast(id)?;
            }
            self.vm_ctxt.import_resolver.parse_to_term(
                &mut self.vm_ctxt.pos_table,
                id,
                format.into(),
            )
        })();
        if let Err(error) = parse_result {
            // Keep the immutable Files entry for diagnostics, but release the
            // registration so callers can correct a syntax error and retry.
            let sources = &mut self.vm_ctxt.import_resolver.sources;
            sources.file_paths.remove(&id);
            sources.file_ids.remove(&canonical_path);
            return Err(self.wrap_error(error.into()));
        }

        let document = source_document(
            &self.vm_ctxt.import_resolver.sources.files,
            SourceId {
                scope: self.source_scope.clone(),
                file: id,
            },
        );
        let parsed = self.vm_ctxt.import_resolver.terms.get(id).unwrap();
        let mut provenance = Vec::new();
        collect_locations(
            parsed,
            &self.vm_ctxt.pos_table,
            &document.id,
            &mut Vec::new(),
            &mut provenance,
        );
        self.registered_sources.push(id);
        Ok(Source {
            document: Arc::new(document),
            format,
            provenance: provenance.into(),
        })
    }

    /// Evaluate a registered source, optionally passing it to a separate Nickel
    /// function. For example, `fun input => input | { count | Number = 1 }`.
    ///
    /// The wrapper's origin is `with_source_name`, or `<wrapper>`. Its imports
    /// resolve independently of the input's imports. This method does not modify
    /// the initial environment or interpolate source names into Nickel code.
    /// The input is cached, but each wrapper call registers a fresh source/AST,
    /// retained until the Context is dropped.
    pub fn eval_with_source(
        &mut self,
        source: &Source,
        wrapper: Option<&str>,
        mode: EvaluationMode,
    ) -> Result<Expr, Error> {
        if !Arc::ptr_eq(&source.document.id.scope, &self.source_scope)
            || !self.registered_sources.contains(&source.document.id.file)
        {
            return Err(invalid_input("invalid source handle for this context"));
        }

        let result = (|| {
            let mut value = self.vm_ctxt.prepare_eval(source.document.id.file)?;
            if let Some(wrapper) = wrapper {
                let wrapper_id = self.vm_ctxt.import_resolver.sources.add_string(
                    SourcePath::Snippet(self.name.as_deref().unwrap_or("<wrapper>").into()),
                    wrapper.to_owned(),
                );
                let function = self.vm_ctxt.prepare_eval(wrapper_id)?;
                value = NickelValue::term_posless(Term::app(function, value));
            }
            let mut vm = VirtualMachine::new(&mut self.vm_ctxt);
            let value = match mode {
                EvaluationMode::Shallow => vm.eval(value)?,
                EvaluationMode::Deep => vm.eval_full(value)?,
                EvaluationMode::DeepForExport => vm.eval_full_for_export(value)?,
            };
            Ok(Expr { value })
        })();
        result.map_err(|error| self.wrap_error(error))
    }
}

fn invalid_input(message: &str) -> Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message).into()
}

fn source_document(files: &Files, id: SourceId) -> SourceDocument {
    SourceDocument {
        name: files.name(id.file).to_owned(),
        text: files.clone_source(id.file),
        id,
    }
}

fn collect_locations(
    value: &NickelValue,
    positions: &PosTable,
    source: &SourceId,
    path: &mut Vec<PathElement>,
    output: &mut Vec<InputLocation>,
) {
    let location = match value.pos(positions) {
        TermPos::Original(span) if span.src_id == source.file => Some(SourceLocation {
            source: source.clone(),
            range: span.start.to_usize()..span.end.to_usize(),
        }),
        _ => None,
    };
    output.push(InputLocation {
        path: path.clone(),
        location,
    });
    let record = value
        .as_record()
        .and_then(|record| record.into_opt())
        .or_else(|| match value.as_term() {
            Some(Term::RecRecord(data)) => Some(&data.record),
            _ => None,
        });
    if let Some(record) = record {
        for (key, field) in &record.fields {
            if let Some(value) = &field.value {
                path.push(PathElement::Field(key.label().to_owned()));
                collect_locations(value, positions, source, path, output);
                path.pop();
            }
        }
    } else if let Some(array) = value.as_array() {
        for (index, value) in array.iter().enumerate() {
            path.push(PathElement::Index(index));
            collect_locations(value, positions, source, path, output);
            path.pop();
        }
    }
}

/// Provisional diagnostic severity, independent of any renderer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Bug,
    Error,
    Warning,
    Note,
    Help,
}

/// An owned label. Its source must be resolved within the same report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticLabel {
    pub location: SourceLocation,
    pub message: String,
    pub primary: bool,
}

/// One diagnostic produced by Nickel's existing conversion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub labels: Vec<DiagnosticLabel>,
    pub notes: Vec<String>,
}

/// Provisional owned report. Only sources referenced by labels are retained.
/// Source text shares the existing Arc allocation; no Context borrow remains.
/// This preserves Nickel's related diagnostics, but does not collect independent
/// failures by recursively forcing other fields: that is an application policy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticReport {
    pub diagnostics: Vec<Diagnostic>,
    pub sources: Vec<SourceDocument>,
}

impl DiagnosticReport {
    pub fn source(&self, id: &SourceId) -> Option<&SourceDocument> {
        self.sources.iter().find(|source| source.id == *id)
    }
}

impl Error {
    /// Convert once against a private mutable snapshot, then obtain sources from
    /// that exact snapshot, including snippets generated during conversion.
    /// Repeated extraction leaves the Error and original Context unchanged.
    pub fn diagnostic_report(&self) -> DiagnosticReport {
        use codespan_reporting::diagnostic::{LabelStyle, Severity as CoreSeverity};
        use nickel_lang_core::error::IntoDiagnostics;

        let mut files = self.files.clone();
        let diagnostics = (*self.error).clone().into_diagnostics(&mut files);
        let synthetic_scope = Arc::new(());
        let mut sources = Vec::<SourceDocument>::new();
        let diagnostics = diagnostics
            .into_iter()
            .map(|diagnostic| {
                let labels = diagnostic
                    .labels
                    .into_iter()
                    .map(|label| {
                        // Existing IDs can be matched back to a registered input.
                        // Generated IDs are local to this conversion, not the Context.
                        let scope =
                            if codespan_reporting::files::Files::source(&self.files, label.file_id)
                                .is_ok()
                            {
                                &self.source_scope
                            } else {
                                &synthetic_scope
                            };
                        let id = SourceId {
                            scope: scope.clone(),
                            file: label.file_id,
                        };
                        if !sources.iter().any(|source| source.id == id) {
                            sources.push(source_document(&files, id.clone()));
                        }
                        DiagnosticLabel {
                            location: SourceLocation {
                                source: id,
                                range: label.range,
                            },
                            message: label.message,
                            primary: label.style == LabelStyle::Primary,
                        }
                    })
                    .collect();
                Diagnostic {
                    severity: match diagnostic.severity {
                        CoreSeverity::Bug => Severity::Bug,
                        CoreSeverity::Error => Severity::Error,
                        CoreSeverity::Warning => Severity::Warning,
                        CoreSeverity::Note => Severity::Note,
                        CoreSeverity::Help => Severity::Help,
                    },
                    code: diagnostic.code,
                    message: diagnostic.message,
                    labels,
                    notes: diagnostic.notes,
                }
            })
            .collect();
        DiagnosticReport {
            diagnostics,
            sources,
        }
    }
}

#[cfg(test)]
mod tests;
