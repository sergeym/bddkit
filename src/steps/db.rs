use crate::world::World;

/// Switches the scenario's current connection. An error if the name is unknown.
pub fn use_connection(w: &mut World, name: &str) -> Result<(), String> {
    w.db.set_current(name)
}

pub fn debug_on(w: &mut World) -> Result<(), String> {
    w.debug = true;
    Ok(())
}

pub fn debug_off(w: &mut World) -> Result<(), String> {
    w.debug = false;
    Ok(())
}
