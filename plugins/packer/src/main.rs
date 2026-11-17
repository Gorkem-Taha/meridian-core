//! The plugin binary: wiring only.

fn main() -> Result<(), meridian_sdk::ServeError> {
    meridian_sdk::serve(
        meridian_sdk::Plugin::new(meridian_target_packer::Packer)
            .with_runner(meridian_target_packer::PackerRunner::from_env()),
    )
}
