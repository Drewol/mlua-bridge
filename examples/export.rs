use mlua_bridge::mlua_bridge;

struct Foo;

#[mlua_bridge]
impl Foo {
    fn func_test() -> u32 {
        5
    }

    fn custom_log(&self, msg: String) {
        self.not_exported();
        println!("From Lua: {msg}");
    }
}

impl Foo {
    fn not_exported(&self) {
        println!("Call to non exported function");
    }
}

fn main() {
    let lua = mlua::Lua::new();

    lua.globals().set("foo", Foo);
    lua.load(
        r#"
x = foo.func_test();
y = string.format("%02d", x);
foo:custom_log(y);
        "#,
    )
    .exec();

    let x: u32 = lua.globals().get("x").expect("x not set");
    assert!(x == 5);
}
