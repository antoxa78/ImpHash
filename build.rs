fn main() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap();
    let secs = now.as_secs();
    let dt = chrono::DateTime::from_timestamp(secs as i64, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=COMPILED_DATE_TIME={}", dt);
}
