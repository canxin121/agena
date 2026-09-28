//! Prints the generated TypeScript state mirror to stdout.
//!
//! ```bash
//! cargo run -p agena-web-types > packages/agena-web/src/generated/agenaState.ts
//! ```

fn main() {
    print!("{}", agena_web_types::module_source());
}
