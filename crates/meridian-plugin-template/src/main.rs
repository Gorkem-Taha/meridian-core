//! The plugin binary: wiring only.

fn main() -> Result<(), meridian_sdk::ServeError> {
    meridian_sdk::serve(
        meridian_sdk::Plugin::new(meridian_plugin_template::Template)
            .with_runner(meridian_plugin_template::TemplateRunner::from_env()),
    )
}
