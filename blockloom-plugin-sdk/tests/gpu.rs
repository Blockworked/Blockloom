//! A plugin's GPU effects are what the world's host reads.

use blockloom_plugin_api::manifest::Capability;
use blockloom_plugin_host::world::Effect;
use blockloom_plugin_sdk::{Error, Host, Plugin, Value, gpu, harness, json};

struct Scaler;

impl Plugin for Scaler {
    fn start(_host: &Host) -> Result<Self, Error> {
        Ok(Scaler)
    }

    fn call_json(&mut self, _host: &Host, op: &str, args: Value) -> Result<Value, Error> {
        match op {
            "scale" => Ok(json!({"effects": [
                gpu::buffer("input", 64),
                gpu::buffer("out", 64),
                gpu::write_f32("input", 0, &[1.0, 2.0]),
                gpu::dispatch("scale", &[("input", "input"), ("output", "out")], [1, 1, 1]),
                gpu::read("out", 0, 2, "scaled", gpu::As::F32),
                gpu::free("input"),
            ]})),
            op if op == gpu::RESULT_OP => {
                let read = gpu::result(&args).ok_or_else(|| Error::bad_argument("a result"))?;
                Ok(json!({"value": read.values.len()}))
            }
            _ => Err(Error::unsupported(op)),
        }
    }
}

#[test]
fn the_effects_a_plugin_builds_are_read_by_the_host() {
    let plugin = harness!(Scaler, [Capability::GpuCompute]).unwrap();
    let answer = plugin.call("scale", json!({})).unwrap();
    let effects = answer["effects"].as_array().unwrap();
    assert_eq!(effects.len(), 6);
    for effect in effects {
        let parsed: Effect = serde_json::from_value(effect.clone()).unwrap();
        assert!(parsed.is_gpu(), "{effect}");
        assert!(parsed.gpu_command().unwrap().is_ok(), "{effect}");
    }
}

#[test]
fn a_finished_read_reaches_the_result_op() {
    let plugin = harness!(Scaler).unwrap();
    let answer = plugin
        .call(
            gpu::RESULT_OP,
            json!({"tag": "scaled", "buffer": "out", "offset": 0, "values": [2.0, 4.0]}),
        )
        .unwrap();
    assert_eq!(answer["value"], 2);
}
