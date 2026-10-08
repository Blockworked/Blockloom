// Minimal C# guest: the same contract as minimal.rs, .c, .py, .mjs and .go.
// Build with csharp/build.sh (see it for the toolchain), then put the .wasm
// under assets/scripts and add it as the actor's script.
using Blockloom;

public static class Guest
{
    // The trimmer wants an entry point to root; the host never calls it.
    public static void Main() { }

    // The one thing a script defines: which Script the host runs.
    public static Script Create() => new Minimal();
}

sealed class Minimal : Script
{
    public override void Start() => World.Say("hello from wasm");

    public override void Tick(float dt)
    {
        World.Read(Verbs.ReadPosition);
        World.Act(Verbs.ActChangePosition, numbers: new double[] { 0, 1 });
    }

    public override void OnEvent(uint kind, double n0, double n1, double n2, double n3)
        => World.Say("event heard");
}
