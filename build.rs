fn main() {
    let now = chrono::Local::now();
    println!(
        "cargo:rustc-env=BUILD_DATE_TIME={}",
        now.format("%Y-%m-%d %H:%M:%S %Z")
    );
}
