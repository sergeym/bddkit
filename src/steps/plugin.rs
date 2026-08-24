use crate::world::World;

pub fn use_instance(w: &mut World, group: &str, name: &str) -> Result<(), String> {
    w.plugins.use_instance(group, name)
}
