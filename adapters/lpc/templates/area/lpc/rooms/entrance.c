inherit "/std/room";

void create() {
    ::create();
    set_short("{{area_name}} Entrance");
    set_long("The entrance to {{area_name}}.");
    add_exit("north", "./rooms/hall");
}
