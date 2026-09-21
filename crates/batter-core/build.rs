// Cargo forwards this source identity to direct dependents, not to the runtime.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!(
        "cargo::metadata=source={}",
        std::env::var("CARGO_MANIFEST_DIR").expect("Cargo supplies the package directory")
    );
}
