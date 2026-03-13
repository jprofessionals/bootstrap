use mud_adapter_sdk::prelude::*;

#[no_mangle]
pub extern "C" fn mud_module_init(registrar: &mut ModuleRegistrar) {
    registrar.set_path("rooms/entrance");
    registrar.set_type(ModuleType::Room);
    registrar.add_dependency("/std/room");
    registrar.register_kfun("title", title);
    registrar.register_kfun("description", description);
    registrar.register_kfun("exits", exits);
}

#[mud_kfun(cacheable)]
fn title(_ctx: &Context, _obj: ObjectId) -> String {
    "The Entrance".into()
}

#[mud_kfun(cacheable)]
fn description(_ctx: &Context, _obj: ObjectId) -> String {
    "Welcome to {{area_name}}. A passage leads north.".into()
}

#[mud_kfun(cacheable)]
fn exits(_ctx: &Context, _obj: ObjectId) -> Vec<Exit> {
    vec![Exit::new("north", "rooms/hall")]
}
