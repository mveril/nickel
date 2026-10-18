# Independent upstream review record

Reviewer: custom `upstream_reviewer`, read only. The reviewer inspected both repositories' relevant sources, all experimental files including untracked files, test/parity/timing outputs, and Clippy output. The reviewer did not independently rerun tests.

Initial critical findings:

1. Confirmed recovery defect: parsing failure reserved the registered path. Fixed by rolling back source lookup/origin indexes while retaining immutable diagnostic text. TOML and Nickel retry tests verify old/new text and identities.
2. A source name establishes filesystem origin and import lookup identity, not just a display label. Preserve the display/origin distinction and obtain upstream agreement on memory shadowing and rejection after prior import.
3. Import lookup still includes format. Registering TOML as data.ncl requires an explicit TOML import. Added mismatch tests and documentation.
4. Source clones copied the location vector and all paths. Changed document and location storage to shared Arc allocations; clone-sharing assertions added.
5. Repeated wrapper calls retain additional source/AST state. Added source-retention test and documented the limitation. No whole-program caching or bounded-memory claim.
6. Public fields and exhaustive provisional data enums need accessors/non-exhaustiveness decisions; SourceId needs a Hash/persistence policy before stabilization.
7. Preserve downstream rebuild/non-finite guards and JSON/Serde semantics. Pending iteration contracts require a separate compatibility decision.
8. Debug timing compares registration with provenance against a baseline without it and supports no superiority claim.

The reviewer found no new unsafe ownership assumption. Context scope and retained identity ownership prevent cross-owner identity reuse; owned reports read sources from the same post-conversion snapshot and distinguish synthetic scopes.

Follow-up inspection confirmed that Arc sharing, retry, format mismatch, disk precedence, search paths, source retention, and nested four-error traversal address prior implementation recommendations. The reviewer saw 33 unit tests plus one doctest pass, clean final Clippy, and a successful executable example. No further confirmed code defect was found.

The follow-up identified a report overclaim about formatter parity tests, which initially checked only messages, label counts and notes. The primary agent strengthened those tests to also compare severity, code, and per-label source ID/range/style/message, then reran the full stable-crate feature test set and Clippy successfully. Final timing results and their limitations appear in the report.

Unresolved architectural discussions: source-name/origin/duplicate policy, eager versus optional location collection, stable report data shapes, opaque identity hashing and report-local synthetic identity, and whether measured wrapper retention justifies a reusable program abstraction.
