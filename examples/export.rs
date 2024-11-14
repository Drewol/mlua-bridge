use mlua::AnyUserData;
use mlua_bridge::mlua_bridge;

struct Foo {
    bar: u32,
}

struct Bar {
    name: &'static str,
}

struct Baz {
    name: String,
}

#[mlua_bridge(rename_funcs = "PascalCase", rename_fields = "camelCase", pub_only)]
impl Foo {
    const X: i32 = 100;
    pub fn func_test() -> u32 {
        5
    }

    pub fn custom_log(&self, msg: String, baz: &mut Baz, context: &Bar) {
        self.not_exported();
        let ctx = context.name;
        baz.name = msg.clone();
        println!("[{ctx}] From Lua: {msg}");
    }

    pub fn set_static_value(v: u32) {
        println!("Static set to {v}")
    }

    pub fn get_static_value() -> u32 {
        5
    }

    pub fn get_bar(&self) -> u32 {
        self.bar
    }

    pub fn set_bar(&mut self, v: u32) {
        self.bar = v;
    }

    fn not_exported(&self) {
        println!("Call to non exported function");
    }

    pub fn get_invalid(x: &Incompatible, v: u32) -> u32 {
        v + x.0
    }

    pub fn set_invalid(x: &Incompatible) -> u32 {
        x.0
    }
}

struct Incompatible(u32);

fn main() {
    let lua = mlua::Lua::new();

    lua.globals()
        .set("foo", Foo { bar: 5 })
        .expect("Failed to set lua global");
    lua.set_app_data(Bar { name: "Foo" });
    lua.set_app_data(Baz {
        name: "Bar".to_owned(),
    });
    lua.load(
        r#"
x = foo.FuncTest();
foo.bar = foo.bar + x + foo.staticValue;
y = string.format("%02d", x);
foo:CustomLog(y);
        "#,
    )
    .exec()
    .expect("Failed to execute lua");
    let foo_lua: AnyUserData = lua.globals().get("foo").expect("foo not set");
    let foo_lua: Foo = foo_lua.take().expect("coult not get userdata");

    let baz: Baz = lua.remove_app_data().unwrap();
    assert!(baz.name == "05");

    let x: u32 = lua.globals().get("x").expect("x not set");
    assert!(x == 5);
    assert!(foo_lua.bar == 15)
}
