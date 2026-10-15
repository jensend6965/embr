//! Compiles the `.slint` file next to the crate that draws it.
//!
//! The generated Rust lands in `OUT_DIR` and is pulled in with
//! `slint::include_modules!()`; it is worth knowing that the *whole* window is drawn from
//! `ui/app.slint`, so an error in that file is a build failure here, with a Slint
//! diagnostic that points at the line.
fn main() {
    slint_build::compile("ui/app.slint").expect("slint build failed");
}
