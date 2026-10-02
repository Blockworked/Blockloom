//! A starting point: a greeter that counts how often it was asked.
//!
//! Ops: `greet` (`name`) answers a line and the count, `count` answers the
//! count. The same source builds as a native library and as a WebAssembly
//! module.

use blockloom_plugin_sdk::{Error, Host, Plugin, Value, export_plugin, json};

#[derive(Default)]
pub struct Starter {
    greeted: i64,
}

impl Plugin for Starter {
    fn start(host: &Host) -> Result<Self, Error> {
        host.info("started");
        Ok(Starter::default())
    }

    fn call_json(&mut self, _host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        match op {
            "greet" => {
                let name = args["name"]
                    .as_str()
                    .filter(|name| !name.is_empty())
                    .ok_or_else(|| Error::bad_argument("name must be some text"))?;
                self.greeted += 1;
                Ok(json!({ "text": format!("Hello, {name}!"), "value": self.greeted }))
            }
            "count" => Ok(json!({ "value": self.greeted })),
            _ => Err(Error::unsupported(op)),
        }
    }
}

export_plugin!(Starter);
