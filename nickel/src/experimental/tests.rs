use super::*;
use crate::ErrorFormat;
use nickel_lang_core::cache::CacheOp;

fn field(name: &str) -> PathElement {
    PathElement::Field(name.into())
}

fn slice<'a>(source: &'a Source, path: &[PathElement]) -> Option<&'a str> {
    let location = source
        .input_locations()
        .iter()
        .find(|entry| entry.path == path)?
        .location
        .as_ref()?;
    assert_eq!(location.source, source.document().id);
    source.document().text.get(location.range.clone())
}

fn json(context: &Context, expression: &Expr) -> serde_json::Value {
    serde_json::from_str(&context.expr_to_json(expression).unwrap()).unwrap()
}

// The same rebuild workaround that RustiQ currently applies to imported data.
const REBUILD: &str = r#"let rec rebuild = fun value =>
    if std.is_record value then std.record.map (fun _ field => rebuild field) value
    else if std.is_array value then std.array.map rebuild value
    else value
in rebuild"#;

fn rebuilt(wrapper: &str) -> String {
    format!("fun input => ({wrapper}) (({REBUILD}) input)")
}

const SCHEMA: &str = r#"fun input => input | {
    name | String,
    calculation | { iterations | Number | default = 50, charge | Number | default = 0 },
}"#;

#[test]
fn toml_composition_defaults_nested_single_and_exports() {
    let mut context = Context::new();
    let source = context
        .add_source(
            "calculation.toml",
            SourceFormat::Toml,
            "name = 'H₂ café'\n[calculation]\ncharge = 2\n",
        )
        .unwrap();
    assert_eq!(source.document().name, "calculation.toml");
    assert_eq!(source.format(), SourceFormat::Toml);
    let expression = context
        .eval_with_source(
            &source,
            Some(&rebuilt(SCHEMA)),
            EvaluationMode::DeepForExport,
        )
        .unwrap();
    assert_eq!(
        json(&context, &expression),
        serde_json::json!({"name": "H₂ café", "calculation": {"charge": 2, "iterations": 50}})
    );
    let toml = context.expr_to_toml(&expression).unwrap();
    assert!(toml.contains("iterations = 50"));
    assert_eq!(
        slice(&source, &[field("calculation"), field("charge")]),
        Some("2")
    );
    assert_eq!(
        slice(&source, &[field("calculation"), field("iterations")]),
        None
    );
}

#[test]
fn batch_and_native_nickel_composition() {
    let wrapper = r#"fun input => std.array.map (fun calculation => calculation | { charge | Number | default = 0 }) input.calculations"#;
    let mut context = Context::new();
    for (name, format, text) in [
        (
            "batch.toml",
            SourceFormat::Toml,
            "[[calculations]]\ncharge = 2\n[[calculations]]\n",
        ),
        (
            "batch.ncl",
            SourceFormat::Nickel,
            "{ calculations = [{ charge = 1 + 1 }, {}] }",
        ),
    ] {
        let source = context.add_source(name, format, text).unwrap();
        let expression = context
            .eval_with_source(&source, Some(wrapper), EvaluationMode::DeepForExport)
            .unwrap();
        assert_eq!(
            json(&context, &expression),
            serde_json::json!([{"charge": 2}, {"charge": 0}])
        );
    }
}

#[test]
fn syntax_and_contract_errors_preserve_input_identity() {
    let mut context = Context::new();
    let syntax = context
        .add_source("broken.toml", SourceFormat::Toml, "name = [")
        .err()
        .unwrap();
    let report = syntax.diagnostic_report();
    validate_report(&report);
    assert!(
        report
            .sources
            .iter()
            .any(|source| source.name == "broken.toml")
    );
    let source = context
        .add_source(
            "wrong.toml",
            SourceFormat::Toml,
            "name = 42\n[calculation]\n",
        )
        .unwrap();
    let error = context
        .eval_with_source(
            &source,
            Some(&rebuilt(SCHEMA)),
            EvaluationMode::DeepForExport,
        )
        .err()
        .unwrap();
    let report = error.diagnostic_report();
    validate_report(&report);
    assert!(
        report
            .diagnostics
            .iter()
            .flat_map(|d| &d.labels)
            .any(|label| label.location.source == source.document().id)
    );
}

#[test]
fn repeated_evaluation_reuses_parsed_input() {
    let mut context = Context::new();
    let source = context
        .add_source("repeat.toml", SourceFormat::Toml, "x = 3\n")
        .unwrap();
    let id = source.document().id.file;
    for _ in 0..3 {
        let value = context
            .eval_with_source(
                &source,
                Some("fun input => input & { y = input.x + 1 }"),
                EvaluationMode::DeepForExport,
            )
            .unwrap();
        assert_eq!(json(&context, &value), serde_json::json!({"x": 3, "y": 4}));
        let positions = context.vm_ctxt.pos_table.clone();
        assert_eq!(
            context
                .vm_ctxt
                .import_resolver
                .parse_to_term(&mut context.vm_ctxt.pos_table, id, InputFormat::Toml)
                .unwrap(),
            CacheOp::Cached(())
        );
        assert_eq!(positions, context.vm_ctxt.pos_table);
    }
    context
        .eval_with_source(&source, None, EvaluationMode::Deep)
        .unwrap();
    let positions = context.vm_ctxt.pos_table.clone();
    for _ in 0..3 {
        context
            .eval_with_source(&source, None, EvaluationMode::Deep)
            .unwrap();
    }
    assert_eq!(
        positions, context.vm_ctxt.pos_table,
        "importer positions would grow if the source were reparsed"
    );
    assert!(Arc::ptr_eq(
        &source.document().text,
        &context.vm_ctxt.import_resolver.sources.clone_source(id)
    ));
}

#[test]
fn duplicates_invalid_and_cross_context_handles() {
    let mut context = Context::new();
    let source = context
        .add_source("duplicate.toml", SourceFormat::Toml, "x = 1")
        .unwrap();
    assert!(
        context
            .add_source("./duplicate.toml", SourceFormat::Toml, "x = 2")
            .is_err()
    );
    assert!(
        context
            .add_source("duplicate.toml", SourceFormat::Nickel, "{}")
            .is_err()
    );
    let mut other = Context::new();
    let alien = other
        .add_source("alien.toml", SourceFormat::Toml, "x = 9")
        .unwrap();
    assert!(
        other
            .eval_with_source(&source, None, EvaluationMode::Deep)
            .is_err()
    );
    assert!(
        context
            .eval_with_source(&alien, None, EvaluationMode::Deep)
            .is_err()
    );
    // Simulate an invalid future C handle without exposing a Rust constructor.
    let mut invalid = alien;
    Arc::make_mut(&mut invalid.document).id.scope = context.source_scope.clone();
    let unused_id = context
        .vm_ctxt
        .import_resolver
        .sources
        .files
        .add("unregistered", "0");
    Arc::make_mut(&mut invalid.document).id.file = unused_id;
    assert!(
        context
            .eval_with_source(&invalid, None, EvaluationMode::Deep)
            .is_err()
    );
    let expression = context
        .eval_with_source(&source, None, EvaluationMode::Deep)
        .unwrap();
    assert_eq!(json(&context, &expression), serde_json::json!({"x": 1}));
}

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        use std::sync::atomic::{AtomicU64, Ordering};
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "nickel-embedding-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn relative_filesystem_imports_and_registered_memory_imports() {
    let directory = Fixture::new();
    directory.write("dependency.ncl", "(import \"numbers.toml\").x + 1");
    directory.write("numbers.toml", "x = 6\n");
    let mut context = Context::new().with_source_name(
        directory
            .0
            .join("wrapper.ncl")
            .to_string_lossy()
            .into_owned(),
    );
    let source = context
        .add_source(
            directory.0.join("input.ncl"),
            SourceFormat::Nickel,
            "{ answer = import \"dependency.ncl\" }",
        )
        .unwrap();
    let value = context
        .eval_with_source(
            &source,
            Some("fun input => input & { extra = import \"dependency.ncl\" }"),
            EvaluationMode::Deep,
        )
        .unwrap();
    assert_eq!(
        json(&context, &value),
        serde_json::json!({"answer": 7, "extra": 7})
    );
    let memory = context
        .add_source(directory.0.join("memory.toml"), SourceFormat::Toml, "x = 8")
        .unwrap();
    let importer = context
        .add_source(
            directory.0.join("importer.ncl"),
            SourceFormat::Nickel,
            "import \"memory.toml\"",
        )
        .unwrap();
    let value = context
        .eval_with_source(&importer, None, EvaluationMode::Deep)
        .unwrap();
    assert_eq!(json(&context, &value), serde_json::json!({"x": 8}));
    assert_eq!(slice(&memory, &[field("x")]), Some("8"));
}

#[test]
fn shallow_and_export_modes_preserve_contract_evaluation_policy() {
    let mut context = Context::new();
    let source = context
        .add_source(
            "lazy.ncl",
            SourceFormat::Nickel,
            "{ good = 1, hidden | not_exported = 1 / 0 }",
        )
        .unwrap();
    assert!(
        context
            .eval_with_source(&source, None, EvaluationMode::Shallow)
            .is_ok()
    );
    assert!(
        context
            .eval_with_source(&source, None, EvaluationMode::DeepForExport)
            .is_ok()
    );
    assert!(
        context
            .eval_with_source(&source, None, EvaluationMode::Deep)
            .is_err()
    );
}

#[test]
fn provenance_nested_inline_arrays_utf8_crlf_and_duplicate_names() {
    let text = "title = 'é🧪'\r\n\"a.b\" = 7\r\ninline = { charge = 2, values = ['λ', 4] }\r\n[a]\r\ncharge = 3\r\n[b]\r\ncharge = 5\r\n[[batch]]\r\ncharge = 6\r\n";
    let mut context = Context::new();
    let source = context
        .add_source("utf8.toml", SourceFormat::Toml, text)
        .unwrap();
    for (path, expected) in [
        (vec![field("title")], "'é🧪'"),
        (vec![field("a.b")], "7"),
        (vec![field("inline"), field("charge")], "2"),
        (
            vec![field("inline"), field("values"), PathElement::Index(0)],
            "'λ'",
        ),
        (
            vec![field("inline"), field("values"), PathElement::Index(1)],
            "4",
        ),
        (vec![field("a"), field("charge")], "3"),
        (vec![field("b"), field("charge")], "5"),
        (
            vec![field("batch"), PathElement::Index(0), field("charge")],
            "6",
        ),
    ] {
        assert_eq!(slice(&source, &path), Some(expected), "{path:?}");
    }
    let value = context
        .eval_with_source(
            &source,
            Some(&rebuilt(
                "fun input => input & { generated | default = 99 }",
            )),
            EvaluationMode::Deep,
        )
        .unwrap();
    assert_eq!(json(&context, &value)["generated"], 99);
    assert_eq!(slice(&source, &[field("generated")]), None);
    assert_eq!(source.document().text.as_ref(), text);
    assert!(
        source
            .input_locations()
            .iter()
            .filter_map(|entry| entry.location.as_ref())
            .all(|location| source.document().text.get(location.range.clone()).is_some())
    );
}

#[test]
fn parsed_provenance_does_not_guess_imported_or_computed_locations() {
    let mut context = Context::new();
    let source = context
        .add_source(
            "computed.ncl",
            SourceFormat::Nickel,
            "{ explicit = 3, computed = 1 + 2 }",
        )
        .unwrap();
    assert_eq!(slice(&source, &[field("explicit")]), Some("3"));
    // The original computation has syntax, but this is not a literal data origin.
    assert_eq!(slice(&source, &[field("computed")]), Some("1 + 2"));
    assert_eq!(slice(&source, &[field("absent")]), None);
    let directory = Fixture::new();
    directory.write("data.toml", "x = 8");
    let imported = context
        .add_source(
            directory.0.join("main.ncl"),
            SourceFormat::Nickel,
            "import \"data.toml\"",
        )
        .unwrap();
    assert_eq!(slice(&imported, &[field("x")]), None);
    let value = context
        .eval_with_source(&imported, None, EvaluationMode::Deep)
        .unwrap();
    let x = value.as_record().unwrap().value_by_name("x").unwrap();
    let span = x.value.pos(&context.vm_ctxt.pos_table).unwrap();
    assert_ne!(span.src_id, imported.document().id.file);
    assert_eq!(
        context
            .vm_ctxt
            .import_resolver
            .sources
            .files
            .source_slice(span),
        "8"
    );
}

#[test]
fn evaluated_positions_are_not_a_reliable_original_input_map() {
    let mut context = Context::new();
    let source = context
        .add_source("origins.toml", SourceFormat::Toml, "explicit = 7")
        .unwrap();
    let value = context
        .eval_with_source(
            &source,
            Some("fun input => input & { defaulted | default = 9, computed = input.explicit + 1 }"),
            EvaluationMode::Deep,
        )
        .unwrap();
    let record = value.as_record().unwrap();
    for key in ["explicit", "defaulted", "computed"] {
        let value = record.value_by_name(key).unwrap();
        let pos = value.value.pos(&context.vm_ctxt.pos_table);
        eprintln!("evaluated {key}: {pos:?}");
        if key != "explicit" {
            assert_eq!(slice(&source, &[field(key)]), None);
            // Even an Original position may refer to wrapper syntax rather than input.
            if let Some(span) = pos.into_opt() {
                assert_ne!(span.src_id, source.document().id.file);
            }
        }
    }
    let missing = NickelValue::number_posless(1);
    assert_eq!(missing.pos(&context.vm_ctxt.pos_table), TermPos::None);
}

fn validate_report(report: &DiagnosticReport) {
    assert!(!report.diagnostics.is_empty());
    for diagnostic in &report.diagnostics {
        assert!(!diagnostic.message.is_empty());
        for label in &diagnostic.labels {
            let source = report
                .source(&label.location.source)
                .expect("every label must resolve against the exact snapshot");
            assert!(
                source.text.get(label.location.range.clone()).is_some(),
                "invalid {:?} for {:?}",
                label.location.range,
                source.text
            );
        }
    }
}

#[test]
fn reports_cover_syntax_contract_utf8_eof_and_formatter_compatibility() {
    for text in [
        "{",
        "{ a = ",
        "{ a = \"é🧪\", b | Number = \"λ\" }",
        "{ a | Number = true, b = 1 }",
    ] {
        let mut context = Context::new().with_source_name("source.ncl".into());
        let error = context.eval_deep(text).err().unwrap();
        let report = error.diagnostic_report();
        validate_report(&report);
        let second = error.diagnostic_report();
        validate_report(&second);
        assert_eq!(report.diagnostics.len(), second.diagnostics.len());
        let mut json = Vec::new();
        error.format(&mut json, ErrorFormat::Json).unwrap();
        let formatted: serde_json::Value = serde_json::from_slice(&json).unwrap();
        assert!(
            formatted.get("sources").is_none(),
            "existing JSON omits source contents"
        );
        for (expected, actual) in formatted["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .zip(&report.diagnostics)
        {
            assert_eq!(expected["message"], actual.message);
            assert_eq!(
                expected["labels"].as_array().unwrap().len(),
                actual.labels.len()
            );
            assert_eq!(expected["notes"], serde_json::json!(actual.notes));
            assert_eq!(actual.severity, Severity::Error);
            assert_eq!(
                expected["severity"],
                serde_json::json!(codespan_reporting::diagnostic::Severity::Error)
            );
            assert_eq!(expected["code"], serde_json::json!(actual.code));
            for (expected_label, label) in expected["labels"]
                .as_array()
                .unwrap()
                .iter()
                .zip(&actual.labels)
            {
                assert_eq!(expected_label["message"], label.message);
                assert_eq!(
                    expected_label["range"],
                    serde_json::json!(label.location.range)
                );
                assert_eq!(
                    expected_label["file_id"],
                    serde_json::json!(label.location.source.file)
                );
                let style = if label.primary {
                    codespan_reporting::diagnostic::LabelStyle::Primary
                } else {
                    codespan_reporting::diagnostic::LabelStyle::Secondary
                };
                assert_eq!(expected_label["style"], serde_json::json!(style));
            }
        }
        for format in [
            ErrorFormat::Text,
            ErrorFormat::AnsiText,
            ErrorFormat::Json,
            ErrorFormat::Yaml,
            ErrorFormat::Toml,
        ] {
            let mut out = Vec::new();
            error.format(&mut out, format).unwrap();
            assert!(!out.is_empty());
        }
        assert!(
            report
                .sources
                .iter()
                .any(|source| source.text.as_ref() == text)
        );
    }
}

#[test]
fn imported_contract_reports_have_multiple_sources_and_labels() {
    let directory = Fixture::new();
    directory.write("bad.ncl", "{ x = \"é\" }");
    let mut context = Context::new();
    let source = context
        .add_source(
            directory.0.join("main.ncl"),
            SourceFormat::Nickel,
            "(import \"bad.ncl\") | { x | Number }",
        )
        .unwrap();
    let error = context
        .eval_with_source(&source, None, EvaluationMode::Deep)
        .err()
        .unwrap();
    let report = error.diagnostic_report();
    validate_report(&report);
    assert!(report.diagnostics.iter().any(|d| d.labels.len() > 1));
    assert!(
        report
            .sources
            .iter()
            .any(|s| s.name == directory.0.join("bad.ncl"))
    );
    assert!(report.sources.iter().any(|s| s.id == source.document().id));
}

#[test]
fn reports_own_shared_sources_after_context_and_error_destruction() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<DiagnosticReport>();
    let report = {
        let mut context = Context::new();
        let source = context
            .add_source("owned.toml", SourceFormat::Toml, "x = 'é'")
            .unwrap();
        let error = context
            .eval_with_source(
                &source,
                Some("fun input => input | { x | Number }"),
                EvaluationMode::Deep,
            )
            .err()
            .unwrap();
        let report = error.diagnostic_report();
        let reported_source = report.source(&source.document().id).unwrap();
        assert!(Arc::ptr_eq(&reported_source.text, &source.document().text));
        report
    };
    validate_report(&report);
    assert!(
        report
            .sources
            .iter()
            .any(|source| source.text.as_ref() == "x = 'é'")
    );
}

#[test]
fn synthetic_sources_are_captured_from_the_converted_snapshot() {
    // Force a failure with a value that has no source position. Core's diagnostic
    // conversion must add a fallback snippet. A source table captured before
    // conversion cannot resolve the new label.
    let mut context = Context::new();
    let contract = context.eval_shallow("fun x => x | Number").unwrap();
    let value = NickelValue::term_posless(Term::app(
        contract.value,
        NickelValue::string_posless("é synthetic"),
    ));
    let error = VirtualMachine::new(&mut context.vm_ctxt)
        .eval_full(value)
        .err()
        .unwrap();
    let error = context.wrap_error(error.into());
    let first = error.diagnostic_report();
    let second = error.diagnostic_report();
    validate_report(&first);
    validate_report(&second);
    let generated = first
        .sources
        .iter()
        .filter(|source| !Arc::ptr_eq(&source.id.scope, &context.source_scope))
        .collect::<Vec<_>>();
    assert!(
        !generated.is_empty(),
        "the fixture must actually exercise synthetic source generation"
    );
    for source in generated {
        assert!(
            second.source(&source.id).is_none(),
            "synthetic identities must not collide across independent conversions"
        );
    }
}

#[test]
fn record_iteration_bypasses_pending_contracts_regression_reproducer() {
    let mut context = Context::new();
    let expression = context
        .eval_shallow("{ foo = 1 } | { foo | std.contract.Equal 2 }")
        .unwrap();
    let record = expression.as_record().unwrap();
    assert!(
        context
            .eval_expr_shallow(record.value_by_name("foo").unwrap())
            .is_err()
    );
    assert!(
        context
            .eval_expr_shallow(record.key_value_by_index(0).unwrap().1.unwrap())
            .is_err()
    );
    let (_, value) = record.iter().next().unwrap();
    assert_eq!(
        context.eval_expr_shallow(value.unwrap()).unwrap().as_i64(),
        Some(1)
    );
}

#[test]
fn collect_independent_errors_using_named_lookup_after_shallow_evaluation() {
    let mut context = Context::new();
    let source = context
        .add_source("many.toml", SourceFormat::Toml, "x = 'wrong'\ny = true")
        .unwrap();
    let wrapper = "fun input => input | { x | Number, y | String }";
    assert!(
        context
            .eval_with_source(&source, Some(wrapper), EvaluationMode::DeepForExport)
            .is_err()
    );
    let expression = context
        .eval_with_source(&source, Some(wrapper), EvaluationMode::Shallow)
        .unwrap();
    let record = expression.as_record().unwrap();
    let mut reports = Vec::new();
    for (name, _) in record.iter() {
        let error = context
            .eval_expr_shallow(record.value_by_name(name).unwrap())
            .err()
            .unwrap();
        reports.push(error.diagnostic_report());
    }
    assert_eq!(reports.len(), 2);
    for report in reports {
        validate_report(&report);
    }
}

#[derive(serde::Deserialize, Debug, PartialEq)]
struct IntegerField {
    value: i64,
}
#[derive(serde::Deserialize, Debug, PartialEq)]
struct Nested {
    inner: IntegerField,
    optional: Option<String>,
}
#[derive(serde::Deserialize, Debug, PartialEq)]
enum Choice {
    Unit,
    Payload(i64),
}

#[test]
fn json_and_direct_serde_fractional_nested_optional_and_enums() {
    let mut context = Context::new();
    let fractional = context.eval_deep("{ value = 1.5 }").unwrap();
    assert!(
        serde_json::from_str::<IntegerField>(&context.expr_to_json(&fractional).unwrap()).is_err()
    );
    assert_eq!(fractional.to_serde::<IntegerField>().unwrap().value, 2);
    for text in [
        "{ inner = { value = 42 } }",
        "{ inner = { value = 42 }, optional = null }",
        "{ inner = { value = 42 }, optional = \"yes\" }",
    ] {
        let value = context.eval_deep(text).unwrap();
        assert_eq!(
            value.to_serde::<Nested>().unwrap(),
            serde_json::from_str::<Nested>(&context.expr_to_json(&value).unwrap()).unwrap()
        );
    }
    let tag = context.eval_deep("'Unit").unwrap();
    assert_eq!(tag.to_serde::<Choice>().unwrap(), Choice::Unit);
    assert_eq!(
        serde_json::from_str::<Choice>(&context.expr_to_json(&tag).unwrap()).unwrap(),
        Choice::Unit
    );
    let variant = context.eval_deep("'Payload 42").unwrap();
    assert_eq!(variant.to_serde::<Choice>().unwrap(), Choice::Payload(42));
    assert!(context.expr_to_json(&variant).is_err());
}

#[test]
fn numeric_boundary_observations() {
    // Catch panics so unsupported direct conversions can be recorded without
    // concealing the rest of the compatibility matrix.
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let mut context = Context::new();
    for text in [
        "-0.5",
        "0.5",
        "1.5",
        "-1.5",
        "9007199254740993",
        "9223372036854775807",
        "9223372036854775808",
        "-9223372036854775809",
        "18446744073709551615",
        "18446744073709551616",
        "170141183460469231731687303715884105727",
    ] {
        let value = context.eval_deep(text).unwrap();
        let exported = context.expr_to_json(&value).unwrap();
        let json = serde_json::from_str::<i64>(&exported);
        let direct = catch_unwind(AssertUnwindSafe(|| value.to_serde::<i64>()));
        eprintln!("number={text} json={exported} json_i64={json:?} direct_i64={direct:?}");
        let expected = match text {
            "-0.5" | "0.5" => 0,
            "1.5" => 2,
            "-1.5" => -2,
            "9007199254740993" => 9007199254740993,
            "-9223372036854775809" => i64::MIN,
            _ => i64::MAX,
        };
        assert_eq!(direct.unwrap().unwrap(), expected);
        assert_eq!(
            json.is_ok(),
            matches!(text, "9007199254740993" | "9223372036854775807")
        );
    }
    let value = context
        .eval_deep("170141183460469231731687303715884105727")
        .unwrap();
    assert_eq!(value.to_serde::<i128>().unwrap(), i128::MAX);
    assert!(serde_json::from_str::<i128>(&context.expr_to_json(&value).unwrap()).is_err());
    let value = context.eval_deep("9223372036854775807").unwrap();
    assert_eq!(value.to_serde::<i64>().unwrap(), i64::MAX);
    assert_eq!(
        serde_json::from_str::<i64>(&context.expr_to_json(&value).unwrap()).unwrap(),
        i64::MAX
    );
}

#[test]
fn non_finite_toml_importer_panics_are_separate_from_embedding_design() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    for special in ["inf", "-inf", "nan"] {
        let result = catch_unwind(AssertUnwindSafe(|| {
            Context::new().add_source(
                "special.toml",
                SourceFormat::Toml,
                format!("value = {special}"),
            )
        }));
        assert!(
            result.is_err(),
            "update the investigation if the importer now rejects {special} structurally"
        );
    }
}

#[test]
fn nested_json_deserialization_contract_regression() {
    let value = Context::new()
        .eval_deep_for_export(
            r#"(std.deserialize 'Json "{\"outer\": {\"x\": 1}}") | { outer | { x | Number } }"#,
        )
        .unwrap();
    assert_eq!(
        value
            .as_record()
            .unwrap()
            .value_by_name("outer")
            .unwrap()
            .as_record()
            .unwrap()
            .value_by_name("x")
            .unwrap()
            .as_i64(),
        Some(1)
    );
}

#[test]
fn toml_eof_ranges_are_valid_and_not_extended_past_source() {
    for text in [
        "value = [",
        "value = 'é'\nother = ",
        "value = @",
        "value = 'λ'\n💥",
    ] {
        let error = Context::new()
            .add_source("bad-utf8.toml", SourceFormat::Toml, text)
            .err()
            .unwrap();
        let report = error.diagnostic_report();
        validate_report(&report);
        assert!(
            report
                .diagnostics
                .iter()
                .flat_map(|d| &d.labels)
                .all(|l| l.location.range.end <= text.len())
        );
    }
}

#[test]
fn native_registration_populates_ast_and_reuses_it_for_preparation() {
    let mut context = Context::new();
    let source = context
        .add_source(
            "native-cache.ncl",
            SourceFormat::Nickel,
            "{ answer = 6 * 7 }",
        )
        .unwrap();
    assert_eq!(
        context
            .vm_ctxt
            .import_resolver
            .parse_to_ast(source.document().id.file)
            .unwrap(),
        CacheOp::Cached(())
    );
    context
        .eval_with_source(&source, None, EvaluationMode::Deep)
        .unwrap();
    let positions = context.vm_ctxt.pos_table.clone();
    context
        .eval_with_source(&source, None, EvaluationMode::Deep)
        .unwrap();
    assert_eq!(context.vm_ctxt.pos_table, positions);
}

#[test]
#[cfg(debug_assertions)]
fn nested_imported_container_contract_assertion_reproducer() {
    use std::panic::{AssertUnwindSafe, catch_unwind};
    let result = catch_unwind(AssertUnwindSafe(|| {
        let mut context = Context::new();
        let input = context
            .add_source("nested.toml", SourceFormat::Toml, "[nested]\nx = 1")
            .unwrap();
        context.eval_with_source(
            &input,
            Some("fun input => input | { nested | { x | Number } }"),
            EvaluationMode::Deep,
        )
    }));
    assert!(
        result.is_err(),
        "remove the downstream rebuild workaround only after investigating a change here"
    );
}

fn core_based_context(name: &str, text: &str, format: SourceFormat) -> Context {
    let mut context = Context::new();
    let id = context.vm_ctxt.import_resolver.sources.add_string(
        SourcePath::Path(name.into(), format.into()),
        text.to_owned(),
    );
    context
        .vm_ctxt
        .import_resolver
        .parse_to_term(&mut context.vm_ctxt.pos_table, id, format.into())
        .unwrap();
    context.vm_ctxt.extend_env.push((
        nickel_lang_core::identifier::Ident::new("Input"),
        NickelValue::term_posless(Term::ResolvedImport(id)),
    ));
    context
}

#[test]
#[ignore = "requires an external read-only RustiQ checkout via RUSTIQ_CHECKOUT"]
fn rustiq_readonly_real_schema_and_fixtures_parity() {
    let root =
        std::path::PathBuf::from(std::env::var_os("RUSTIQ_CHECKOUT").expect("set RUSTIQ_CHECKOUT"));
    let schema = std::fs::read_to_string(root.join("src/runfile/nickel/calculation.ncl")).unwrap();
    let rebuild =
        std::fs::read_to_string(root.join("src/runfile/nickel/rebuild-data.ncl")).unwrap();
    let resolve = std::fs::read_to_string(root.join("src/runfile/nickel/resolve.ncl"))
        .unwrap()
        .replace("import \"calculation.ncl\"", &format!("({schema})"));
    let wrapper = format!("fun input => ({resolve}) (({rebuild}) input)");
    let sample = std::fs::read_to_string(root.join("samples/h2/sto-3g/calculation.toml")).unwrap();
    let cases = [
        (SourceFormat::Toml, sample, true),
        (
            SourceFormat::Toml,
            "[basis]\nname = 'sto-3g'\n".to_owned(),
            true,
        ),
        (
            SourceFormat::Toml,
            "[basis]\nname = 'sto-3g'\n[method.hf.diis]\nenabled = true\nmax_history = 8"
                .to_owned(),
            true,
        ),
        (
            SourceFormat::Toml,
            "[basis]\nname = 'sto-3g'\n[molecule]\ncharge = 2147483648".to_owned(),
            false,
        ),
        (
            SourceFormat::Toml,
            "[basis]\nname = 'sto-3g'\n[method.hf]\nmax_iterations = 1.5".to_owned(),
            false,
        ),
        (
            SourceFormat::Nickel,
            "[{ basis.name = \"sto-3g\" }, { basis.name = \"6-31g\", method.mp2 = {} }]".to_owned(),
            true,
        ),
        (SourceFormat::Nickel, "[]".to_owned(), false),
    ];
    for (index, (format, text, valid)) in cases.into_iter().enumerate() {
        let mut core = core_based_context("core-input", &text, format);
        let core_value = core.eval_deep_for_export(&format!("({wrapper}) Input"));
        let mut context = Context::new();
        let source = context
            .add_source(format!("parity-{index}"), format, text)
            .unwrap();
        let experimental =
            context.eval_with_source(&source, Some(&wrapper), EvaluationMode::DeepForExport);
        assert_eq!(core_value.is_ok(), valid, "case {index}");
        assert_eq!(experimental.is_ok(), valid, "case {index}");
        if let (Ok(core_value), Ok(value)) = (core_value, experimental) {
            assert_eq!(
                json(&core, &core_value),
                json(&context, &value),
                "case {index}"
            );
        }
    }
}

#[test]
#[ignore = "reproducible timing probe; run alone with --ignored --nocapture --test-threads=1"]
fn embedding_timings() {
    use std::{hint::black_box, time::Instant};
    const N: usize = 100;
    let text = (0..200)
        .map(|i| format!("field_{i} = {i}\n"))
        .collect::<String>();
    let wrapper = "fun input => input & { extra | default = 42 }";
    eprintln!(
        "timing profile={} iterations={N} input_bytes={} fields=200",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        text.len()
    );
    for sample in 0..5 {
        let mut context = Context::new();
        let start = Instant::now();
        for i in 0..N {
            black_box(
                context
                    .add_source(format!("registration-{i}.toml"), SourceFormat::Toml, &text)
                    .unwrap(),
            );
        }
        eprintln!(
            "sample={sample} operation=experimental_register_parse_provenance total_us={}",
            start.elapsed().as_micros()
        );
        let mut core = Context::new();
        let start = Instant::now();
        for i in 0..N {
            let id = core.vm_ctxt.import_resolver.sources.add_string(
                SourcePath::Path(format!("registration-{i}.toml").into(), InputFormat::Toml),
                text.clone(),
            );
            core.vm_ctxt
                .import_resolver
                .parse_to_term(&mut core.vm_ctxt.pos_table, id, InputFormat::Toml)
                .unwrap();
        }
        eprintln!(
            "sample={sample} operation=core_register_parse_no_provenance total_us={}",
            start.elapsed().as_micros()
        );
        let mut context = Context::new();
        let source = context
            .add_source("timed.toml", SourceFormat::Toml, &text)
            .unwrap();
        let mut core = core_based_context("core-timed.toml", &text, SourceFormat::Toml);
        let core_wrapper = format!("({wrapper}) Input");
        let expected = core.eval_deep_for_export(&core_wrapper).unwrap();
        let actual = context
            .eval_with_source(&source, Some(wrapper), EvaluationMode::DeepForExport)
            .unwrap();
        assert_eq!(json(&core, &expected), json(&context, &actual));
        let start = Instant::now();
        for _ in 0..N {
            black_box(
                context
                    .eval_with_source(&source, Some(wrapper), EvaluationMode::DeepForExport)
                    .unwrap(),
            );
        }
        eprintln!(
            "sample={sample} operation=experimental_repeated_wrapper_deep_export total_us={}",
            start.elapsed().as_micros()
        );
        let start = Instant::now();
        for _ in 0..N {
            black_box(core.eval_deep_for_export(&core_wrapper).unwrap());
        }
        eprintln!(
            "sample={sample} operation=core_repeated_wrapper_deep_export total_us={}",
            start.elapsed().as_micros()
        );
        let start = Instant::now();
        for _ in 0..N {
            black_box(
                context
                    .eval_with_source(&source, None, EvaluationMode::Shallow)
                    .unwrap(),
            );
        }
        eprintln!(
            "sample={sample} operation=experimental_repeated_shallow total_us={}",
            start.elapsed().as_micros()
        );
        let start = Instant::now();
        for _ in 0..N {
            black_box(
                context
                    .eval_with_source(&source, None, EvaluationMode::DeepForExport)
                    .unwrap(),
            );
        }
        eprintln!(
            "sample={sample} operation=experimental_repeated_deep_export total_us={}",
            start.elapsed().as_micros()
        );
        let error = context
            .eval_deep("{ x | Number = \"wrong\" }")
            .err()
            .unwrap();
        let start = Instant::now();
        for _ in 0..N {
            black_box(error.diagnostic_report());
        }
        eprintln!(
            "sample={sample} operation=owned_diagnostic_snapshot total_us={}",
            start.elapsed().as_micros()
        );
    }
}

#[test]
fn failed_registration_can_be_corrected_without_changing_old_diagnostics() {
    for (name, format, bad, good) in [
        ("retry.toml", SourceFormat::Toml, "x = [", "x = 7"),
        ("retry.ncl", SourceFormat::Nickel, "{ x = ", "{ x = 7 }"),
    ] {
        let mut context = Context::new();
        let error = context.add_source(name, format, bad).err().unwrap();
        let source = context.add_source(name, format, good).unwrap();
        let value = context
            .eval_with_source(&source, None, EvaluationMode::Deep)
            .unwrap();
        assert_eq!(json(&context, &value), serde_json::json!({"x": 7}));
        let old = error.diagnostic_report();
        validate_report(&old);
        let old_source = old.sources.iter().find(|s| s.name == name).unwrap();
        assert_eq!(old_source.text.as_ref(), bad);
        assert_ne!(old_source.id, source.document().id);
        assert_eq!(source.document().text.as_ref(), good);
    }
}

#[test]
fn explicit_format_is_preserved_even_when_name_extension_disagrees() {
    let directory = Fixture::new();
    let mut context = Context::new();
    let input = context
        .add_source(directory.0.join("data.ncl"), SourceFormat::Toml, "x = 8")
        .unwrap();
    let explicit = context
        .add_source(
            directory.0.join("explicit.ncl"),
            SourceFormat::Nickel,
            "import \"data.ncl\" as 'Toml",
        )
        .unwrap();
    let inferred = context
        .add_source(
            directory.0.join("inferred.ncl"),
            SourceFormat::Nickel,
            "import \"data.ncl\"",
        )
        .unwrap();
    for source in [&input, &explicit] {
        let value = context
            .eval_with_source(source, None, EvaluationMode::Deep)
            .unwrap();
        assert_eq!(json(&context, &value), serde_json::json!({"x": 8}));
    }
    assert!(
        context
            .eval_with_source(&inferred, None, EvaluationMode::Deep)
            .is_err()
    );
    let cloned = input.clone();
    assert!(Arc::ptr_eq(&input.document, &cloned.document));
    assert!(Arc::ptr_eq(&input.provenance, &cloned.provenance));
}

#[test]
fn collect_nested_and_array_errors_without_skipping_deferred_contracts() {
    fn collect(context: &mut Context, expression: Expr, reports: &mut Vec<DiagnosticReport>) {
        match context.eval_expr_shallow(expression) {
            Err(error) => reports.push(error.diagnostic_report()),
            Ok(value) => {
                if let Some(record) = value.as_record() {
                    for (name, _) in record.iter() {
                        if let Some(field) = record.value_by_name(name) {
                            collect(context, field, reports);
                        }
                    }
                } else if let Some(array) = value.as_array() {
                    for value in array.iter() {
                        collect(context, value, reports);
                    }
                }
            }
        }
    }
    let mut context = Context::new();
    let input = context.add_source("nested-errors.toml", SourceFormat::Toml,
        "[settings]\nx = 'bad'\ny = false\n[[calculations]]\ncharge = 'wrong'\n[[calculations]]\ncharge = true").unwrap();
    let wrapper = rebuilt(
        "fun input => input | { settings | { x | Number, y | Number }, calculations | Array { charge | Number } }",
    );
    assert!(
        context
            .eval_with_source(&input, Some(&wrapper), EvaluationMode::DeepForExport)
            .is_err()
    );
    let shallow = context
        .eval_with_source(&input, Some(&wrapper), EvaluationMode::Shallow)
        .unwrap();
    let mut reports = Vec::new();
    collect(&mut context, shallow, &mut reports);
    assert_eq!(reports.len(), 4);
    for report in &reports {
        validate_report(report);
        assert!(
            report
                .sources
                .iter()
                .any(|source| source.id == input.document().id)
        );
    }
}

#[test]
fn memory_registration_shadows_disk_but_does_not_replace_an_existing_import() {
    let directory = Fixture::new();
    directory.write("data.toml", "x = 1");
    let mut context = Context::new();
    let memory = context
        .add_source(directory.0.join("data.toml"), SourceFormat::Toml, "x = 2")
        .unwrap();
    let importer = context
        .add_source(
            directory.0.join("main.ncl"),
            SourceFormat::Nickel,
            "import \"data.toml\"",
        )
        .unwrap();
    let value = context
        .eval_with_source(&importer, None, EvaluationMode::Deep)
        .unwrap();
    assert_eq!(json(&context, &value), serde_json::json!({"x": 2}));
    assert_eq!(memory.document().text.as_ref(), "x = 2");
    assert_eq!(
        std::fs::read_to_string(directory.0.join("data.toml")).unwrap(),
        "x = 1"
    );
    let mut other = Context::new();
    let disk_importer = other
        .add_source(
            directory.0.join("other.ncl"),
            SourceFormat::Nickel,
            "import \"data.toml\"",
        )
        .unwrap();
    let value = other
        .eval_with_source(&disk_importer, None, EvaluationMode::Deep)
        .unwrap();
    assert_eq!(json(&other, &value), serde_json::json!({"x": 1}));
    assert!(
        other
            .add_source(directory.0.join("data.toml"), SourceFormat::Toml, "x = 3")
            .is_err()
    );
}

#[test]
fn import_search_paths_work_with_registered_sources() {
    let directory = Fixture::new();
    directory.write("search-only.ncl", "42");
    let mut context =
        Context::new().with_added_import_paths(vec![directory.0.clone().into_os_string()]);
    let source = context
        .add_source(
            "search-main.ncl",
            SourceFormat::Nickel,
            "import \"search-only.ncl\"",
        )
        .unwrap();
    let value = context
        .eval_with_source(&source, None, EvaluationMode::Deep)
        .unwrap();
    assert_eq!(value.as_i64(), Some(42));
}

#[test]
fn wrappers_are_retained_but_reports_do_not_mutate_context_sources() {
    let mut context = Context::new();
    let source = context
        .add_source("retention.toml", SourceFormat::Toml, "x = 1")
        .unwrap();
    let before = context
        .vm_ctxt
        .import_resolver
        .sources
        .files
        .filenames()
        .count();
    for _ in 0..3 {
        context
            .eval_with_source(&source, Some("fun input => input"), EvaluationMode::Deep)
            .unwrap();
    }
    assert_eq!(
        context
            .vm_ctxt
            .import_resolver
            .sources
            .files
            .filenames()
            .count(),
        before + 3
    );
    assert_eq!(context.registered_sources.len(), 1);
    let error = context
        .eval_deep("{ x | Number = \"wrong\" }")
        .err()
        .unwrap();
    let before = context
        .vm_ctxt
        .import_resolver
        .sources
        .files
        .filenames()
        .count();
    for _ in 0..3 {
        validate_report(&error.diagnostic_report());
    }
    assert_eq!(
        context
            .vm_ctxt
            .import_resolver
            .sources
            .files
            .filenames()
            .count(),
        before
    );
}
