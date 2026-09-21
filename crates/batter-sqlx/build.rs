// The unpublished native integration checks the foundation Cargo actually chose.
// Forward core too: a consumer patch must not silently select a second source.
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!(
        "cargo::metadata=source={}",
        std::env::var("CARGO_MANIFEST_DIR").expect("Cargo supplies the package directory")
    );
    println!(
        "cargo::metadata=core_source={}",
        std::env::var("DEP_BATTER_CORE_FOUNDATION_SOURCE")
            .expect("batter-core supplies its actual Cargo source")
    );
}
