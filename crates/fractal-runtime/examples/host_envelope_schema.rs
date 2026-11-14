fn main() -> Result<(), serde_json::Error> {
    println!(
        "{}",
        serde_json::to_string_pretty(&fractal_runtime::host_envelope::json_schema())?
    );
    Ok(())
}
