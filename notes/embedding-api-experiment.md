# Stable Nickel embedding: experimental results

## 1. Executive summary

Named explicit-format inputs, composition with a separately evaluated Nickel function, original parsed-input locations, and owned diagnostic reports are feasible without exposing `FileId`, `PosTable`, position indices, or VM types. The implementation reuses Nickel's existing parser/importer, caches, evaluator, exporters, and diagnostic conversion. It adds no dependency and no virtual filesystem. These are experimental APIs behind the non-default `experimental-embedding` feature, not finalized stable additions.

The strongest recommendation is to extend the existing `Context` with opaque source registration/composition, provide parsed syntax locations, and add an owned generic diagnostic report. A second stable program owner or builder is unnecessary for the demonstrated use case. Evaluated-expression provenance is not ready for a public guarantee.

The migration is **not yet ready to ship**. Existing core behavior includes a nested TOML-container contract assertion, pending contracts bypassed by record iteration, non-finite TOML importer panics, and direct Serde rounding/saturation that differs from JSON deserialization. RustiQ's rebuild wrapper, non-finite preflight, named field lookup during error collection, and JSON/Serde boundary must remain. Only the separate TOML diagnostic range defect was corrected here.

### Isolation and reproducibility

| Item | Recorded state |
| --- | --- |
| Nickel checkout | `/home/mveril/git/nickel`, clean `master`, `9fe00ea6107d0218d55ffd3d05a0c225b6bdb5cd` |
| RustiQ checkout | `/home/mveril/git/RustiQ`, clean `main`, `5b5f376b4671a6237970b013e364fc48618284c1` |
| Experimental worktree | `/tmp/nickel-embedding-prototypes` |
| Experimental branch | `experiment/embedding-api-prototypes`, same base commit, uncommitted changes |
| Toolchain | Rust/Cargo 1.99.0, x86_64 Linux, Intel Core i7-8700 @ 3.20GHz |
| Instructions | No Nickel/ancestor `AGENTS.md` found; RustiQ root instructions read and respected |

Both repositories were clean before work, so there was no uncommitted work to move or overwrite. All implementation and report changes live in the experimental worktree. RustiQ was read only, including the real-schema parity probe. No commit, push, issue publication, or PR was created. The task requires no RustiQ book edit because it changes no RustiQ code or behavior.

The two requested audit agents ran concurrently and finished before implementation. The independent reviewer ran after working prototypes, targeted tests, a real-schema parity run, and preliminary timing measurements. The primary agent made every implementation change.

## 2. Prototype results

### A. Explicit-format source composition

**Objective:** parse a named TOML buffer once, retain its name/format, then pass it through a separate Nickel expression that supplies contracts, defaults, and normalization.

**Implementation:** `Context::add_source` registers an immutable input in the existing `SourceCache`, parses it through `CacheHub`, and returns opaque `Source`. Nickel input populates the AST cache first, avoiding an initial parse-to-term fallback followed by a second parse at preparation. TOML uses the existing importer and term cache. `eval_with_source` prepares that input, optionally prepares a wrapper function as another source, constructs an application internally, and invokes the existing shallow/deep/deep-for-export VM operation. There is no generated import string and no mutation of the initial environment.

`SourceId` privately contains an `Arc` scope and internal file identity. Equality requires both, preventing accidental cross-context matches. Safe Rust cannot construct a source handle. Evaluation validates both scope and registered membership; an internal test simulates an invalid future handle. Cloned handles share their document and location snapshot through `Arc`. Dropping a handle does not unregister its input. Dropping the context releases the evaluator independently of surviving source documents.

Names have two roles in this prototype. `Files` keeps the exact original `OsString` for display. Import origins and cache lookup keys use lexical absolute paths resolved at registration; symlinks are not canonicalized. This requires private cache index wiring, which should move behind a core helper if upstream accepts the policy. Ordinary relative filesystem imports, configured import search paths, and registered memory imports use the existing resolver. The input and wrapper have separate import origins; `with_source_name` sets the wrapper's origin. Registration does not honor `NICKEL_IMPORT_PATH` automatically, consistent with the existing embedding context.

Duplicates of a normalized path are rejected, including differing formats and paths previously loaded by filesystem import. A memory registration made first can shadow a file without changing the disk file. A syntax error releases the registration name for a corrected retry while retaining the old immutable text for diagnostics. This deliberately differs from core's last-name-wins registration behavior and is a policy requiring maintainer agreement.

Explicit format is preserved independently of the extension. A TOML registration named `data.ncl` works through its handle and through `import "data.ncl" as 'Toml`; inferred Nickel imports do not select the TOML cache entry by name alone. The prototype does not change Nickel's `(path, format)` import rules.

**Modified files:** `nickel/Cargo.toml`, `nickel/src/lib.rs`, new `nickel/src/experimental.rs`, `nickel/src/experimental/tests.rs`, and `nickel/examples/embedding_prototype.rs`. Most implementation is in the new module; stable context/error changes only supply feature-gated identity state and the module declaration.

**Tests and results:** valid TOML and Nickel, syntax failures, contracts, defaults, nested records, single/batch input, repeated evaluation, duplicate/invalid/cross-context handles, retry after parse failure, relative/nested filesystem imports, search paths, memory-versus-disk precedence, extension/format mismatch, UTF-8 values, and shallow versus export forcing pass. Cache-hit and unchanged position-table assertions demonstrate reuse of the input parser result. The full example runs. The opt-in downstream probe reads RustiQ's actual calculation/resolve/rebuild sources and sample input, compares JSON with its core-based registration/binding pattern, and verifies seven successful/invalid single/batch cases. It passes without writing to RustiQ.

**Limitations:** nested imported TOML records still need RustiQ's existing recursive rebuild before receiving contracts in debug builds. A separate reproducer catches the original core assertion; the library does not hide it. Repeated wrapper calls create and retain fresh wrapper sources and ASTs. Input caching is not whole-program caching or bounded-memory execution. Package-map configuration, source replacement/removal, and compiled wrapper reuse were not added.

### A alternatives: handle versus public builder

| Concern | Context-owned opaque handle (implemented) | Small stable program/builder (evaluated against existing core implementation) |
| --- | --- | --- |
| Ownership | Existing Context owns VM/caches; Source owns shared immutable metadata | A new program would own another VM/cache lifecycle or borrow Context |
| Lifetimes | No Context borrow in Source; evaluation checks ownership dynamically | Owning program can statically contain its inputs; a borrowed builder complicates lifetimes |
| Identity | Context-scoped private token; exact original name separate from origin | Builder-local identity simpler until values must return to Context for further forcing |
| Duplicates | Explicit immutable/reject policy | Must define merge order, replacement, and duplicate naming too |
| Invalid/cross-context use | Checked and tested | Avoidable by a self-contained program, but cross-owner Expr use remains a design question |
| Imports | Existing resolver, distinct wrapper/input origins; explicit format rules unchanged | Existing core multi-input builder synthesizes imports and merges inputs |
| Compatibility | Additive methods and opaque types on the existing entry point | Introduces a second stable owner and Expr/evaluator interoperability questions |
| Performance | Input reused; new wrapper preparation and eager location snapshot cost | Could naturally reuse an owned wrapper, but no comparative builder benchmark was run |
| C API | Separate checked source registry can mirror the handle model | Owned opaque program pointer is possible; adds another destruction/lifecycle API |

Core `ProgramBuilder::add_source_with_format` already exists. It does not by itself implement arbitrary wrapper-function composition into stable `Expr`. Its multi-input path merges sources and uses the temporary `%inmem_src%:` import prefix from issue [#2362](https://github.com/nickel-lang/nickel/issues/2362). Local commits `4134c916fe3b6c8c26a9dea6631f7cfbe25dc8c2` and `4b722502bc1a7c99b1a9d0ead9fed9ac465762df`, the current prefix resolver, and `core/tests/integration/program_api.rs` establish the regression and hotfix. The latter commit explicitly describes native multiple-main-input support as the longer-term direction. The issue page could not be fetched; this report does not infer its current status. Neither prototype exposes that prefix. The existing builder regression was included in targeted core tests.

**Recommendation:** handles on Context for this contribution; defer a new program abstraction until reusable wrapper preparation or additional ownership requirements justify it with evidence.

### B. Original parsed-input locations

**Objective:** identify the original source and exact zero-based half-open UTF-8 byte range of explicitly supplied values without exposing position tables.

**Implementation:** registration walks the original parsed literal containers before preparation and snapshots optional locations. Paths are vectors of `Field(String)` and `Index(usize)`, distinguishing repeated leaf names, arrays, and literal keys containing dots. Locations contain an opaque `SourceId` and `Range<usize>`; the source document supplies its original name and shared contents. Only `TermPos::Original` positions from that registered document are accepted. Missing positions remain `None`. The root/structural tables may have different location availability from scalar values; callers must treat every location as optional.

**Tests:** nested tables, inline tables, arrays and arrays of tables, duplicate leaf names, dotted keys, UTF-8 including four-byte characters, CRLF, explicit versus default fields, computed Nickel syntax, imported values, and positionless values pass. Snapshots remain independent of later normalization. A defaulted field absent in input has no parsed-input entry. A Nickel computation can have an original *syntax* range; that is not proof of the computed result's data lineage. Parsed provenance does not recursively evaluate imports or invent locations for imported fields.

**Expression alternative, tested internally:** after evaluating `explicit = 7` with a wrapper that adds defaults and computation, the explicit value retains an original input span; the defaulted `9` has an `Original` span in the **wrapper**; the computed value has an `Inherited` wrapper span. A positionless value has none. An imported TOML value can refer to another source entirely. Therefore `Original` alone is insufficient to identify explicit application input.

An evaluated accessor also needs an ownership guarantee that current `Expr` lacks: it contains neither a context identity nor a position table. A naive `Context::location(&Expr)` could use another context's unrelated index or panic. No such public accessor was added. The internal probe examines values only with their known originating context. General expression provenance would require owner tracking throughout Expr and borrowed container extraction, and guarantees about inherited/generated locations. That is a larger contribution.

**Recommendation:** expose parsed syntax locations, initially with a clear TOML-data guarantee and deliberately limited Nickel literal-container behavior. Do not promise semantic lineage of evaluated values. Decide eager versus optional collection before stabilizing; the current snapshot allocates one path vector per value and clones each path component, although cloning Source shares the resulting snapshot.

### C. Owned structured diagnostics

**Objective:** return diagnostic data with all sources needed to interpret labels, usable after Context destruction.

**Existing API assessment:** `Error::format(ErrorFormat::Json)` already returns diagnostic severity, code, messages, labels/ranges, and notes. Tests verify those fields against the new conversion. Its output omits source names and contents. Raw IDs cannot reliably be reconstructed by callers, especially for synthetic sources. Existing JSON alone does not satisfy this requirement. No formatter was removed or redesigned.

**Implementation:** provisional `Error::diagnostic_report()` clones the existing persistent `Files` snapshot, converts the error exactly once against that mutable clone, and then obtains every labeled source from that same mutated snapshot. It returns generic owned diagnostic/label/source structures. Only referenced sources are retained; their `Arc<str>` text is shared. Strings for messages/notes and source names are allocated by conversion or the owned report. Synthetic sources receive a fresh scope for each extraction so their local IDs cannot collide with another extraction or later Context source. Existing source IDs remain comparable with registered input IDs.

Reports are `Send + Sync`, independent of Context's thread restrictions. They hold no VM or position-table reference. Tests drop both Context and Error before using a report. No `miette` dependency was added; downstream adapters can use any diagnostic library. Multiple diagnostics generated from one core error are preserved. Collecting independent failures in sibling fields remains an application policy; tests demonstrate shallow recursive traversal using **named lookup**, including four failures across nested records and arrays.

**Tests:** TOML and Nickel syntax errors, contracts, multiple labels, cross-file failures, UTF-8, EOF, generated fallback snippets, repeated extraction, report lifetime, Arc source sharing, and text/ANSI/JSON/YAML/TOML formatter compatibility pass. A source-positionless argument deliberately forces generated diagnostic source creation; the test proves the fixture exercises it and checks every label against the converted snapshot. Diagnostic extraction does not add files to the Context.

**Separate defect fixed:** `ParseError::from_toml` added one byte to `toml_edit`'s already half-open end. At EOF that produced an invalid range, and near UTF-8 it could also extend incorrectly. `parser/src/error.rs` now preserves the parser range unchanged. A parser-level regression compares exact ranges for syntax/UTF-8/EOF inputs. This fix should be contributed separately from the report API.

**Remaining questions:** prefer private fields with accessors or non-exhaustive data types before stabilization; decide `SourceId` hashing/serialization and report-local synthetic identity guarantees; avoid promising durable numeric source IDs across processes. Report serialization itself is not implemented. The existing formatter JSON schema is unchanged apart from the corrected TOML ranges.

### Additional correctness investigations

| Investigation | Observed behavior | Consequence |
| --- | --- | --- |
| Deferred contracts | `{ foo = 1 } \| { foo \| std.contract.Equal 2 }`, evaluated shallowly: named/indexed access fails when forced; iteration yields `1` successfully | Existing inconsistency appears to be an implementation bug; maintainers must decide contract and compatibility. Use iterator names plus `value_by_name`, not iterated values, for downstream error forcing. No iteration semantics changed here. |
| Fractional integer conversion | `±0.5` → direct Serde `0`; `1.5` → `2`; `-1.5` → `-2`; JSON-to-i64 rejects each | Direct Serde rounds rather than preserving JSON validation behavior. |
| Integer boundaries | `2^53+1` and `i64::MAX` export and deserialize exactly as i64 on both paths; `i64::MAX+1` is rejected by JSON-to-i64 but direct Serde saturates at max; `i64::MIN-1` saturates at min directly | Direct Serde is not a compatible migration shortcut. |
| Large integers | `u64::MAX` exports exactly as a JSON integer, but both it and larger values saturate when directly requested as i64. Beyond i64/u64 export support, Nickel JSON approximates with f64; direct i128 preserves `i128::MAX`, while JSON-to-i128 rejects the exported scientific notation | Preserve RustiQ's JSON boundary; choose any later numeric policy independently. |
| Enums | `'Unit` converts on both paths; `'Payload 42` converts directly to a Rust payload enum but JSON export rejects it | Direct Serde changes accepted representations. |
| Nested/options | Tested nested integer records and absent/null/present optional strings agree | Agreement is limited to tested shapes; it does not undo numeric/enum differences. |
| Non-finite TOML | `inf`, `-inf`, `nan` each panic in exact rational conversion, not a structured located error | Keep RustiQ's preflight. Issue [#2675](https://github.com/nickel-lang/nickel/issues/2675) remains a separate importer issue; its page was unavailable during this run. No IEEE special values or numeric semantics were introduced. |
| Imported containers | Minimal nested JSON contract regression passes, but nested TOML contract application can hit `eval/fixpoint.rs`'s `value.is_constant()` debug assertion | Keep the existing recursive rebuild. Passing one JSON case does not establish that all deserialization cases are fixed. |

The tests that assert current panic/bypass behavior are investigation reproducers, not desired permanent semantics. When those independent bugs are fixed, replace them with the desired-behavior regressions in their respective contributions.

### Validation and performance evidence

Final commands and captured results are recorded in `embedding-api-evidence/`. The final stable-crate feature run passed **33 unit tests and one doctest**; two opt-in probes were explicitly run and passed separately. The default-feature run passed four unit tests and one doctest. The exact parser regression and both core program/builder regressions passed. Formatting, `git diff --check`, Clippy with all stable-crate features/targets and warnings denied, and the executable example passed. Run from the experimental worktree with Rust 1.99.0 available. Cargo dependencies were already available locally, so checks use `--offline --locked`.

```sh
cargo fmt --all -- --check
cargo test -p nickel-lang --all-features --offline --locked
cargo test -p nickel-lang --offline --locked
cargo test -p nickel-lang-parser --lib --offline --locked toml_error_preserves_half_open_utf8_and_eof_ranges
cargo test -p nickel-lang-core --test integration --offline --locked program_api
cargo clippy -p nickel-lang --all-targets --all-features --offline --locked -- -D warnings
cargo run -p nickel-lang --example embedding_prototype --features experimental-embedding --offline --locked
RUSTIQ_CHECKOUT=/home/mveril/git/RustiQ cargo test -p nickel-lang --features experimental-embedding --offline --locked rustiq_readonly_real_schema_and_fixtures_parity -- --ignored --nocapture
cargo test -p nickel-lang --features experimental-embedding --offline --locked embedding_timings -- --ignored --nocapture --test-threads=1
```

The RustiQ parity and timing probes are opt-in because one requires another repository and the other measures time. They were explicitly run, not skipped as substitutes for validation. The full Nickel workspace and RustiQ's scientific/CLI suites were not run. RustiQ was audited read only; the parity probe executes entirely on the Nickel side and only reads RustiQ files. Full downstream suites remain migration gates.

Timing uses five samples of 100 operations on one 2,980-byte, 200-field TOML document, in the debug profile. It compares registration/import with eager location capture against a core registration/import baseline **without** location capture, and warmed repeated wrapper/deep evaluation against the core `ResolvedImport` initial-environment path. The same JSON is asserted before timing. It also measures shallow versus deep forcing and owned diagnostic snapshots. Exact input bytes and total microseconds are in the evidence. No release throughput, heap allocation count, peak RSS, confidence interval, or end-to-end scientific workload result is claimed.

Final isolated debug timing run (five samples, 100 operations each; medians are sample totals divided by 100):

| Operation | Median µs/op | Observed min–max µs/op |
| --- | ---: | ---: |
| Experimental registration + parse + provenance | 1276.90 | 1257.90–1344.14 |
| Core registration + parse, no provenance | 1102.55 | 1093.21–1146.69 |
| Experimental warmed wrapper + deep export | 2139.42 | 2120.33–2145.70 |
| Core warmed wrapper + deep export | 2186.87 | 2139.42–2218.27 |
| Experimental registered input shallow | 35.80 | 33.65–36.33 |
| Experimental registered input deep export | 1311.79 | 1295.75–1347.18 |
| Owned diagnostic snapshot | 23.03 | 22.06–31.05 |

The registration comparison does not isolate provenance cost: it also includes identity, duplicate/origin bookkeeping, and document metadata. The repeated wrapper results are close in this run and do not establish superiority or equivalence outside this workload.

Direct structural evidence is stronger than these timings: repeated input preparation has unchanged positions and cache hits; source text and cloned source metadata share Arc allocations; three wrapper calls retain three additional sources; diagnostic extraction leaves Context sources unchanged. Registration still copies the caller's text into core-owned storage, builds line offsets and provenance paths, scans existing source paths for duplicate checks, and stores an identity token. Deep evaluation must traverse exported containers. Report conversion still allocates owned metadata, and synthetic snippets require new text. These costs should be measured in release at realistic study sizes before establishing a budget or adding reusable-program APIs.

## 3. Recommended public API

### Existing, experimental, and proposed surfaces

- **Existing stable:** `Context::new`, import paths, `with_source_name`, shallow/deep/export evaluation of Nickel text, `eval_expr_shallow`, `Expr`/record/array inspection, exporters, `Expr::to_serde`, and `Error::format`.
- **Implemented experimental:** `Context::add_source`, `Context::eval_with_source`, `Source`/`SourceFormat`/`SourceId`, source documents and parsed locations, `EvaluationMode`, and `Error::diagnostic_report`, exposed only with `experimental-embedding`.
- **Future proposal:** promote only these demonstrated capabilities after policy review. Keep handles/identity opaque; use private fields/accessors or non-exhaustive report types, and add `Eq + Hash` for source identity. Initially expose Nickel and TOML formats; decide expansion without requiring callers to depend on core's format enum. No stable evaluated provenance method, public VM/builder, arbitrary global binding API, or VFS is proposed.

A minimal candidate signature set, **proposed rather than released**, is:

```rust,ignore
impl Context {
    pub fn add_source(
        &mut self,
        name: impl Into<std::ffi::OsString>,
        format: SourceFormat,
        text: impl Into<String>,
    ) -> Result<Source, Error>;

    pub fn eval_with_source(
        &mut self,
        input: &Source,
        wrapper: Option<&str>, // Nickel function; None evaluates the input itself
        mode: EvaluationMode,
    ) -> Result<Expr, Error>;
}
impl Source {
    pub fn document(&self) -> &SourceDocument;
    pub fn format(&self) -> SourceFormat;
    pub fn input_locations(&self) -> &[InputLocation];
}
impl Error {
    pub fn diagnostic_report(&self) -> DiagnosticReport;
}
```

Keeping the three evaluation modes mirrors existing Context behavior and supports RustiQ's shallow recovery traversal. A mandatory wrapper or dedicated method per evaluation mode might be more idiomatic; that is a naming/surface decision, not a demonstrated need for more machinery. Do not stabilize provenance's eager allocation policy accidentally through an unrelated source-registration PR.

### Complete runnable experimental usage

This example uses implemented experimental methods and existing stable JSON export. All added types are imported from the explicitly experimental namespace:

```rust
use nickel_lang::{Context, experimental::{EvaluationMode, PathElement, SourceFormat}};

fn main() {
    let mut context = Context::new();
    let input = context.add_source(
        "calculation.toml", SourceFormat::Toml, "charge = 2\n",
    ).unwrap();
    let value = context.eval_with_source(
        &input,
        Some("fun input => input | { charge | Number, iterations | Number | default = 50 }"),
        EvaluationMode::DeepForExport,
    ).unwrap();
    let normalized: serde_json::Value = serde_json::from_str(
        &context.expr_to_json(&value).unwrap(),
    ).unwrap();
    assert_eq!(normalized["iterations"], 50);

    let charge = [PathElement::Field("charge".into())];
    let location = input.input_locations().iter()
        .find(|entry| entry.path == charge)
        .and_then(|entry| entry.location.as_ref()).unwrap();
    assert_eq!(&input.document().text[location.range.clone()], "2");

    let error = context.eval_with_source(
        &input, Some("fun input => input | { charge | String }"),
        EvaluationMode::DeepForExport,
    ).err().unwrap();
    let report = error.diagnostic_report();
    drop(error);
    drop(context);
    for diagnostic in &report.diagnostics {
        for label in &diagnostic.labels {
            let source = report.source(&label.location.source).unwrap();
            assert!(source.text.get(label.location.range.clone()).is_some());
        }
    }
}
```

`nickel/examples/embedding_prototype.rs` is the executable nested-record version, including the existing RustiQ rebuild workaround. A future stable usage would retain this workflow with settled stable imports and accessor names; it should still export to JSON before RustiQ's Serde DTOs. This report intentionally does not pretend that future methods can already compile without the feature.

## 4. Compatibility analysis

**Backward compatibility:** the feature is off by default and existing method signatures remain unchanged. The existing Rust C API feature compiles with all features. Generated headers, C consumers, and ABI behavior were not separately exercised. The TOML range correction intentionally changes erroneous label endpoints while preserving formatter structure. Changing record iteration behavior is a separate compatibility decision.

**Source ownership and context lifetime:** Context owns parsed/prepared/evaluation state. Source owns shared immutable text/locations and a scope token, with no Context lifetime parameter. Reports own the labeled source snapshots and can be used after both Context and Error destruction. Scope identity survives as long as references need it, preventing reuse collisions. This is identity within a process, not persistence across serialization or contexts.

**Imports:** name, origin, and lookup identity must be documented separately. The implementation resolves lexical absolute origins once and preserves exact display names. It retains normal format inference, search paths, and file freshness behavior, without generalizing it into a VFS. Memory shadowing/rejection of already imported paths, duplicate policy across formats, symlinks, case-insensitive filesystems, package imports, and source replacement need explicit decisions. Windows path behavior was not exercised.

**Diagnostics:** sources must always come from the same post-conversion snapshot as labels. Generic reports accommodate `miette`, codespan, LSP adapters, or custom renderers without importing those libraries into Nickel. Original input identity can be matched reliably; synthetic identity is report-local and intentionally distinct between extractions. Independent error collection should remain downstream policy unless a separate use case establishes a core API.

**Performance:** registration eagerly parses and captures locations, so it does more than adding a buffer. Repeated input evaluation avoids parsing again but repeated wrapper text still grows retained sources/ASTs. Source cloning shares metadata after the review correction. Source text sharing is asserted, not inferred from a timing result. No new dependency or numeric representation was added. Release measurements and memory budgets remain migration requirements.

**C interoperability:** opaque source/report handles fit the existing C owner model. Rust's Source validation does not make a dangling C pointer valid. A future C registry must check owner, liveness, and generation before accessing a handle; provide explicit destroy functions, borrowed UTF-8 pointer/length accessors, byte offsets, and report-owned label/source lookup. Do not expose core IDs or assume a Rust Arc layout is ABI-stable. No new C functions are part of this prototype.

## 5. Independent review

`upstream_reviewer` inspected both relevant downstream sources and all tracked/untracked prototype code, tests, parity output, timing output, and a successful initial Clippy log. The review was read only and did not independently rerun tests.

**Confirmed defect fixed:** failed parsing originally left the name reserved. Rollback now removes lookup/origin entries, retains immutable diagnostic files, and allows same-name corrected input. TOML/Nickel retry tests assert old and new text and identities independently.

**Other feedback addressed:** Source cloning originally copied every provenance path. The handle now shares both document and provenance using Arc, with pointer-sharing assertions. Added tests/documentation for mismatched filename extensions, memory shadowing versus prior filesystem imports, configured search paths, and retained wrapper sources. Documented wrapper allocation and clarified parsed syntax locations versus semantic lineage. Final checks were rerun after changes. The follow-up review identified an overbroad formatter-comparison claim; the test now also checks severity, code, and each label's range, source ID, style, and message against the existing JSON formatter for the error fixtures.

**No further confirmed implementation defect or new unsafe ownership assumption was identified.** The reviewer agreed that the owned diagnostics use the correct converted snapshot and that a Context handle is the smaller abstraction for the demonstrated downstream requirements.

**Architectural questions left for upstream:** filesystem meaning assigned to a source name; duplicate/format/override policy; eager versus optional locations; private/accessor versus public report data shapes; Eq/Hash and synthetic identity promises; reusable wrapper preparation if memory measurements justify it. The reviewer rejected performance-superiority conclusions from debug timings with a baseline that omits provenance. These remain explicit review topics, not hidden implementation guarantees.

## 6. Upstream contribution plan

Submit independent changes only after maintainer discussion; none is published here.

| Proposed PR title | Scope and affected modules | Tests/documentation | Dependencies and acceptance criteria |
| --- | --- | --- | --- |
| Preserve TOML diagnostic half-open byte ranges | `parser/src/error.rs` only | Exact importer spans, EOF, invalid token, UTF-8; document corrected endpoints in change note | Independent. No out-of-bounds or non-boundary ranges for the fixtures; existing formatters work. |
| Add explicit-format source registration and composition to Context | `nickel/src/lib.rs`, small source module; possibly a private `SourceCache` registration helper | Formats, wrapper functions, defaults/contracts, cache reuse, invalid/cross-owner handles, duplicate/retry policy, origins/search paths, memory/disk precedence, #2362 regression; Rust and C lifecycle discussion | Independent of owned reports/provenance once identity ownership is agreed. Existing API behavior unchanged, no magic prefix exposed, real-schema JSON parity passes. Resolve naming and import policy first. |
| Expose original parsed input locations | Source/provenance module and importer integration | Nested/inline TOML, arrays/tables, UTF-8/CRLF, literal dotted keys, duplicate leaves, absent/default/computed/imported/positionless cases; explicit syntactic guarantee and allocation costs | Depends on source identity. No raw positions/IDs, no default provenance fabricated, capture before transformations. Set eager/optional policy using release measurements. |
| Return owned diagnostics with referenced source snapshots | Error/report module, source identity adapter | Synthetic snippets, repeated extraction, multi-label/import errors, EOF/UTF-8, matching old formatters, drop Context/Error, Send/Sync and text sharing | Requires identity shape and preferably TOML range fix; can use report-local identity if submitted first. Every label resolves in its own report; no miette dependency. |
| Resolve pending record contracts consistently across access APIs | Separate `RecordIter` investigation in stable library | Minimal bypass reproducer, intended named/indexed/iterated equivalence; public compatibility/release documentation | Maintainer decision required before changing behavior. Separate from embedding additions. |
| Return located errors for unsupported non-finite TOML values | TOML importer and parse errors | `inf`, `-inf`, `nan`, nested/array variants and precise spans, no panic | Separate #2675 follow-up. Preserve exact rational semantics; no IEEE special values. |
| Repair nested imported-container contract preparation | Core transform/closurization/fixpoint modules, narrowly scoped after diagnosis | Minimal TOML assertion plus nested/array/JSON contract and dependent field regressions; demonstrate successful output in debug/release | Independent correctness work. Do not remove assertions or silently rebuild all values in a new stable API. Remove RustiQ workaround only after broad parity. |

Keep serialization parity findings as documentation/tests or a separately agreed deserializer policy contribution. Do not bundle new integer conversion semantics into embedding work. Consider compiled wrapper reuse only after a realistic release/memory benchmark demonstrates its need.

## 7. GitHub discussion draft

**Title:** Stable embedding support for named TOML inputs, source locations, and owned diagnostics

RustiQ embeds Nickel for configuration contracts, defaults, normalized single/batch calculations, and source-aware errors. It currently pins `nickel-lang-core = 0.19.0` because stable `nickel-lang` can evaluate Nickel strings but cannot register a named TOML buffer and use it inside a separate Nickel normalization expression. Evaluating TOML alone would not cover this workflow.

Core already has explicit-format sources in `ProgramBuilder::add_source_with_format`, source/term caches, TOML value positions, and diagnostic conversion. An isolated experimental prototype reuses those facilities through the stable embedding layer rather than introducing another importer or a VFS:

- Context-scoped opaque source registration plus applying a separately evaluated Nickel function to that input.
- Parsed-input syntax locations using opaque source identity and half-open UTF-8 byte ranges, captured before defaults/contracts.
- An owned report pairing generic diagnostics with the exact sources from the post-conversion snapshot, including generated snippets.

Tests demonstrate cached input reuse, contracts/defaults, nested/single/batch input, imports, source ownership, cross-context rejection, UTF-8/CRLF ranges, and synthetic diagnostic consistency. A read-only comparison against RustiQ's actual schema and existing core-based composition produced the same JSON for tested valid cases and rejected the tested invalid cases. Existing JSON diagnostic output lacks the source data required for a complete external adapter.

The experiment keeps RustiQ's rebuild/non-finite guards and JSON/Serde conversion. It separately reproduces the record-iteration pending-contract discrepancy and importer panics. It also found a small TOML diagnostic endpoint correction suitable for an independent fix. Debug timing probes establish no throughput advantage; reusable wrapper preparation and release memory measurements remain open.

Would maintainers prefer Context-owned opaque sources over a second stable program/builder owner? We would appreciate agreement on the distinction between display names and import origins, duplicate/format/override policy, optional versus eager parsed locations, and report-local synthetic source identity. We propose small independent contributions for source composition, parsed locations, and owned reports, with unrelated evaluator/importer fixes kept separate. No public position table, raw FileId, miette dependency, or virtual filesystem is proposed.

## 8. RustiQ migration checklist

- [ ] Obtain upstream-reviewed and released stable source composition, parsed locations, and owned source-complete diagnostics; the experimental feature is not a supported migration target.
- [ ] Settle immutable name/duplicate policy, display name versus origin, format inference, memory/disk precedence, filesystem/package imports, and source-handle error behavior.
- [ ] Replace the private core VM/cache wrapper with stable Context; remove direct `CacheHub`, `CacheImpl`, `VmContext`, `InputFormat`, `SourcePath`, `ResolvedImport`, and binding manipulation.
- [ ] Preserve the exact inlined schema, resolution function, recursive rebuild, successful deep-for-export pass, and shallow recovery policy. Pass Input through a function instead of an internal global binding.
- [ ] Replace source-map extraction from `PosTable`/`NickelValue` with parsed input paths. Exact UTF-8 source slices for explicitly supplied scientific fields must match current tests, including CRLF, nested/inline tables, arrays, and duplicate leaf names. Defaulted/computed fields must not receive invented input spans.
- [ ] Adapt owned report labels to miette using only that report's sources; retain severity, notes, multiple labels, imported files, and related diagnostics. Check source identities rather than name equality. Test reports after dropping the context and repeated extraction with synthetic sources.
- [ ] Collect independent errors using stable shallow evaluation and named lookup; recurse through record names and array elements without losing deferred contracts. Reproduce all four existing invalid-field spans and nested/batch failures.
- [ ] Retain JSON export followed by Serde DTO deserialization. Verify fractional integer rejection, bounds, i64/u64 and scientific numeric domains, enum tags/variants, null/optional/nested fields, and domain-error classification. Do not substitute `to_serde()`.
- [ ] Retain the preflight that rejects non-finite TOML values with spans until a no-panic located importer error is released. Preserve exact numeric semantics.
- [ ] Retain the rebuild workaround until nested imported records/arrays pass the real schema in debug and release, with invalid and dependent-field cases. A single passing JSON regression is insufficient.
- [ ] Reproduce native relative/nested imports, imported TOML files, search paths, standalone installed-binary behavior, and top-level geometry path handling. Geometry resolution remains RustiQ's responsibility.
- [ ] Preserve JSON/TOML output and ordering. The current custom record rebuilding for section order may need a Rust-side layout step or a separately agreed stable value-construction facility; it is not covered by the new registration/report APIs alone. Compare existing output fixtures before deleting that core usage.
- [ ] Run all existing frontend/diagnostic/source-map/CLI/schema/output regression suites and core scientific suites from a separately authorized writable downstream checkout; assess documentation changes there.
- [ ] Establish release-profile latency and memory baselines for representative small, large, and parameterized studies. Require no repeated parsing of registered input, no source-text copies per label, no extra successful-resolution pass, and no unexplained regression outside an agreed budget. Track wrapper source/AST retention and drop/reset contexts per bounded work unit until reuse is supported.
- [ ] Measure location collection and owned-report allocation on actual source sizes/error counts. Define a numeric performance budget from those baseline measurements rather than inventing an acceptable percentage here.
- [ ] Audit every remaining `nickel_lang_core` import, including custom ordered TOML export, before replacing the dependency. The parity probe verifies the embedding boundary, not RustiQ's full DTO/domain/output pipeline.
