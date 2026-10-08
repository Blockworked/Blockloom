// Minimal Kotlin guest: the same contract as minimal.rs, .c, .py, .mjs, .go
// and .cs. Build with kotlin/build.sh (see it for the toolchain), then put the
// .wasm under assets/scripts and add it as the actor's script.
private class Minimal : Script() {
    override fun start() = World.say("hello from wasm")

    override fun tick(dt: Float) {
        World.read(Verbs.READ_POSITION)
        World.act(Verbs.ACT_CHANGE_POSITION, numbers = doubleArrayOf(0.0, 1.0))
    }

    override fun onEvent(kind: Int, n0: Double, n1: Double, n2: Double, n3: Double) =
        World.say("event heard")
}

// The one thing a script defines: which Script the host runs.
fun createScript(): Script = Minimal()

// Kotlin/Wasm wants an entry point; the host never calls it.
fun main() {}
