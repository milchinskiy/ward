pub mod minijinja;

/// Initializes the `template` module.
///
/// This module is a namespace for multiple template engines.
///
/// # Errors [`mlua::Error`]
pub fn define(lua: &mlua::Lua) -> mlua::Result<mlua::Table> {
    let template = lua.create_table()?;

    #[allow(clippy::single_element_loop)]
    for (name, module) in [("minijinja", minijinja::define(lua)?)] {
        template.set(name, module.clone())?;
        lua.register_module(format!("ward.template.{name}").as_str(), module)?;
    }

    Ok(template)
}
