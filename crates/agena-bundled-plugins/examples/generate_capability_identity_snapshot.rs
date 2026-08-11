//! Regenerate the committed capability identity snapshot without building the
//! full application binary:
//!
//! ```bash
//! cargo run -p agena-bundled-plugins --example generate_capability_identity_snapshot \
//!   > crates/agena-bundled-plugins/generated/bundled-capability-identities.json
//! ```

fn main() {
    print!(
        "{}",
        agena_bundled_plugins::bundled_capability_identity_snapshot_json()
    );
}
