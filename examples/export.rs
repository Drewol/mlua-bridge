use mlua::AnyUserData;
use mlua_bridge::mlua_bridge;

struct Foo {
    bar: u32,
}

#[mlua_bridge]
impl Foo {
    fn func_test() -> u32 {
        5
    }

    fn custom_log(&self, msg: String) {
        self.not_exported();
        println!("From Lua: {msg}");
    }

    fn set_static_value(v: u32) {
        println!("Static set to {v}")
    }

    fn get_static_value() -> u32 {
        5
    }

    fn get_bar(&self) -> u32 {
        self.bar
    }

    fn set_bar(&mut self, v: u32) {
        self.bar = v;
    }
}

impl Foo {
    fn not_exported(&self) {
        println!("Call to non exported function");
    }
}

fn main() {
    let lua = mlua::Lua::new();

    lua.globals()
        .set("foo", Foo { bar: 5 })
        .expect("Failed to set lua global");
    lua.load(
        r#"
x = foo.func_test();
foo.bar = foo.bar + x + foo.static_value;
y = string.format("%02d", x);
foo:custom_log(y);
        "#,
    )
    .exec()
    .expect("Failed to execute lua");
    let foo_lua: AnyUserData = lua.globals().get("foo").expect("foo not set");
    let foo_lua: Foo = foo_lua.take().expect("coult not get userdata");
    let x: u32 = lua.globals().get("x").expect("x not set");
    assert!(x == 5);
    assert!(foo_lua.bar == 15)
}
