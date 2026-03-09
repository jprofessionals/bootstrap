inherit "/std/room";

void create() {
    ::create();
    set_short("{{area_name}} Hall");
    set_long("A grand hall within {{area_name}}.");
    add_exit("south", "./rooms/entrance");
}
