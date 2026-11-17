//! The plugin binary: wiring only.

fn main() -> Result<(), meridian_sdk::ServeError> {
    meridian_sdk::serve(
        meridian_sdk::Plugin::new(meridian_target_arm::Arm)
            .with_runner(meridian_target_arm::ArmRunner::from_env()),
    )
}
