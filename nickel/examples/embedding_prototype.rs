//! Run with `cargo run -p nickel-lang --example embedding_prototype
//! --features experimental-embedding`. All added APIs are provisional.
use nickel_lang::{
    Context,
    experimental::{EvaluationMode, PathElement, SourceFormat},
};

fn main() {
    let mut context = Context::new();
    let input = context
        .add_source(
            "calculation.toml",
            SourceFormat::Toml,
            "name = 'H₂'\n[calculation]\ncharge = 2\n",
        )
        .unwrap();
    // Keep RustiQ's existing workaround for nested imported containers until
    // the independent core fixpoint assertion has been addressed upstream.
    let wrapper = r#"
        let rec rebuild = fun value =>
            if std.is_record value then
                std.record.map (fun _ field => rebuild field) value
            else if std.is_array value then std.array.map rebuild value
            else value
        in fun input => (rebuild input) | {
            name | String,
            calculation | {
                charge | Number | default = 0,
                iterations | Number | default = 50,
            },
        }
    "#;
    let value = context
        .eval_with_source(&input, Some(wrapper), EvaluationMode::DeepForExport)
        .unwrap();
    // Existing APIs: preserve the JSON/Serde boundary during migration.
    let exported = context.expr_to_json(&value).unwrap();
    let normalized: serde_json::Value = serde_json::from_str(&exported).unwrap();
    assert_eq!(normalized["calculation"]["iterations"], 50);
    println!("{exported}");
    let path = [
        PathElement::Field("calculation".into()),
        PathElement::Field("charge".into()),
    ];
    let location = input
        .input_locations()
        .iter()
        .find(|entry| entry.path == path)
        .and_then(|entry| entry.location.as_ref())
        .unwrap();
    assert_eq!(&input.document().text[location.range.clone()], "2");
    let error = context
        .eval_with_source(
            &input,
            Some("fun input => input.name | Number"),
            EvaluationMode::DeepForExport,
        )
        .err()
        .unwrap();
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
