// Minimal Go (TinyGo) guest: the same contract as minimal.rs, .c, .py and
// .mjs. Rename this to main.go in a module (path example.com/guest) that
// requires go.bytecodealliance.org, with blockloom-script-guest/wit as ./wit:
//
//	go tool wit-bindgen-go generate --world script --out internal ./wit
//	tinygo build -target=wasm-unknown -o core.wasm .
//	wasm-tools component embed wit core.wasm --world script -o embedded.wasm
//	wasm-tools component new embedded.wasm -o minimal.wasm
//
// wasm-unknown keeps WASI out of the component (wasip2 would import wasi:cli,
// which the world does not declare). Then put minimal.wasm under
// assets/scripts and add it as the actor's script.
package main

import (
	"example.com/guest/internal/blockloom/script/acts"
	"example.com/guest/internal/blockloom/script/entry"
	"example.com/guest/internal/blockloom/script/sensors"
	"go.bytecodealliance.org/cm"
)

// abi.rs verb numbers (blockloom-core/src/script/abi.rs).
const (
	readPosition      = 1
	actChangePosition = 3
	actSay            = 10
)

func say(text string) {
	acts.Act(actSay, text, "", "", cm.ToList([]float64{}))
}

func init() {
	entry.Exports.Start = func() { say("hello from wasm") }
	entry.Exports.Tick = func(dt float32) {
		sensors.Read(readPosition, "", "", 0)
		acts.Act(actChangePosition, "", "", "", cm.ToList([]float64{0, 1}))
	}
	entry.Exports.Frame = func(dt float32) {}
	entry.Exports.UI = func(dt float32) {}
	entry.Exports.Stop = func() {}
	entry.Exports.Destroy = func() {}
	entry.Exports.OnEvent = func(kind uint32, n0, n1, n2, n3 float64) { say("event heard") }
}

func main() {}
