# Experimental validation evidence

These are captured outputs from the isolated worktree. See the parent report for exact reproduction commands, scope, and limitations. Empty `format.log` indicates formatting success; `git diff --check` also succeeded. `git-state.txt` records original checkouts and the uncommitted experimental branch.

- `tests-all-features.log`: 33 unit tests, two opt-in tests ignored in this invocation, one doctest.
- `rustiq-parity.log`, `timings.log`: the two opt-in tests explicitly executed and passed.
- `tests-default.log`: four existing unit tests and one doctest.
- `parser-range.log`: exact half-open TOML span regression.
- `core-program.log`: both existing program API regressions, including #2362.
- `clippy.log`: stable crate, all targets/features, warnings denied.
- `example.log`: runnable nested-input, locations, owned-report example.
- `numeric.log`, `provenance.log`: detailed observations from focused passing tests.
- `baseline.log`: clean-checkout stable library baseline.

The expected importer/assertion reproducer panics are caught inside tests; passing those reproducers establishes current limitations, not successful handling of those inputs. Timing is a debug probe, not a production performance guarantee. No release benchmark or heap profiler was run.
