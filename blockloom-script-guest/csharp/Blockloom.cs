// The managed half of the C# guest: a typed face over the world's imports and
// the dispatcher the shim calls. Keep it beside Guest.cs in one assembly.
using System;
using System.Runtime.InteropServices;
using System.Text;

namespace Blockloom;

/// <summary>Override what you need; every call is one entry point of the world.</summary>
public abstract class Script
{
    public virtual void Start() { }
    public virtual void Tick(float dt) { }
    public virtual void Frame(float dt) { }
    public virtual void Ui(float dt) { }
    public virtual void Stop() { }
    public virtual void Destroy() { }
    /// <param name="kind">An abi EVENT_* number.</param>
    public virtual void OnEvent(uint kind, double n0, double n1, double n2, double n3) { }
}

/// <summary>Raw verbs, numbered as blockloom-core/src/script/abi.rs.</summary>
public static class Verbs
{
    public const uint ReadPosition = 1;
    public const uint ActMove = 1;
    public const uint ActGoTo = 2;
    public const uint ActChangePosition = 3;
    public const uint ActSay = 10;
    public const uint ActLog = 19;
}

static class Native
{
    const string Lib = "shim";
    [DllImport(Lib)] public static extern int bl_read(uint what, string a, string b, double arg, out double result);
    [DllImport(Lib)] public static extern unsafe int bl_read_text(uint what, string a, string b, byte* buf, int cap);
    [DllImport(Lib)] public static extern unsafe void bl_act(uint what, string a, string b, string c, double* numbers, int count);
    [DllImport(Lib)] public static extern int bl_state_get_number(string key, out double result);
    [DllImport(Lib)] public static extern unsafe int bl_state_get_text(string key, byte* buf, int cap);
    [DllImport(Lib)] public static extern void bl_state_set_number(string key, double value);
    [DllImport(Lib)] public static extern void bl_state_set_text(string key, string value);
    [DllImport(Lib)] public static extern void bl_state_clear(string key);
    [DllImport(Lib)] public static extern double bl_var_get_number(string name);
    [DllImport(Lib)] public static extern unsafe int bl_var_get_text(string name, byte* buf, int cap);
    [DllImport(Lib)] public static extern void bl_var_set_number(string name, double value);
    [DllImport(Lib)] public static extern void bl_var_set_text(string name, string value);
    [DllImport(Lib)] public static extern int bl_list_len(string name);
    [DllImport(Lib)] public static extern int bl_list_get_number(string name, int index, out double result);
    [DllImport(Lib)] public static extern unsafe int bl_list_get_text(string name, int index, byte* buf, int cap);
    [DllImport(Lib)] public static extern void bl_list_add_number(string name, double value);
    [DllImport(Lib)] public static extern void bl_list_add_text(string name, string value);
    [DllImport(Lib)] public static extern void bl_list_insert_number(string name, int index, double value);
    [DllImport(Lib)] public static extern void bl_list_insert_text(string name, int index, string value);
    [DllImport(Lib)] public static extern void bl_list_replace_number(string name, int index, double value);
    [DllImport(Lib)] public static extern void bl_list_replace_text(string name, int index, string value);
    [DllImport(Lib)] public static extern void bl_list_delete(string name, int index);
    [DllImport(Lib)] public static extern void bl_list_clear(string name);

    /// <summary>Runs a text-returning import, growing the buffer until the whole string fits.</summary>
    public static unsafe string? Text(Func<IntPtr, int, int> call)
    {
        var size = 256;
        while (true)
        {
            var buf = new byte[size];
            fixed (byte* p = buf)
            {
                var n = call((IntPtr)p, size);
                if (n < 0) return null;
                if (n <= size) return Encoding.UTF8.GetString(buf, 0, n);
                size = n;
            }
        }
    }
}

/// <summary>Snapshot reads and world writes, with the verbs as plain numbers.</summary>
public static class World
{
    /// <summary>A number read by abi READ_* verb. Null when the world has no such thing.</summary>
    public static double? Read(uint what, string a = "", string b = "", double arg = 0)
        => Native.bl_read(what, a, b, arg, out var v) == 0 ? v : null;

    public static string? ReadText(uint what, string a = "", string b = "")
        => Native.Text((p, cap) => { unsafe { return Native.bl_read_text(what, a, b, (byte*)p, cap); } });

    public static unsafe void Act(uint what, string a = "", string b = "", string c = "", params double[] numbers)
    {
        fixed (double* n = numbers) Native.bl_act(what, a, b, c, n, numbers.Length);
    }

    public static void Say(string text) => Act(Verbs.ActSay, text);
    public static void Log(string text) => Act(Verbs.ActLog, text);
}

/// <summary>Per-actor state that lasts the run.</summary>
public static class State
{
    public static double? Number(string key) => Native.bl_state_get_number(key, out var v) == 0 ? v : null;
    public static string? Text(string key)
        => Native.Text((p, cap) => { unsafe { return Native.bl_state_get_text(key, (byte*)p, cap); } });
    public static void Set(string key, double value) => Native.bl_state_set_number(key, value);
    public static void Set(string key, string value) => Native.bl_state_set_text(key, value);
    public static void Clear(string key) => Native.bl_state_clear(key);
}

/// <summary>Project variables, by name.</summary>
public static class Vars
{
    public static double Number(string name) => Native.bl_var_get_number(name);
    public static string Text(string name)
        => Native.Text((p, cap) => { unsafe { return Native.bl_var_get_text(name, (byte*)p, cap); } }) ?? "";
    public static void Set(string name, double value) => Native.bl_var_set_number(name, value);
    public static void Set(string name, string value) => Native.bl_var_set_text(name, value);
}

/// <summary>Project lists, by name.</summary>
public static class Lists
{
    public static int Count(string name) => Native.bl_list_len(name);
    public static double? Number(string name, int index) => Native.bl_list_get_number(name, index, out var v) == 0 ? v : null;
    public static string? Text(string name, int index)
        => Native.Text((p, cap) => { unsafe { return Native.bl_list_get_text(name, index, (byte*)p, cap); } });
    public static void Add(string name, double value) => Native.bl_list_add_number(name, value);
    public static void Add(string name, string value) => Native.bl_list_add_text(name, value);
    public static void Insert(string name, int index, double value) => Native.bl_list_insert_number(name, index, value);
    public static void Insert(string name, int index, string value) => Native.bl_list_insert_text(name, index, value);
    public static void Replace(string name, int index, double value) => Native.bl_list_replace_number(name, index, value);
    public static void Replace(string name, int index, string value) => Native.bl_list_replace_text(name, index, value);
    public static void Delete(string name, int index) => Native.bl_list_delete(name, index);
    public static void Clear(string name) => Native.bl_list_clear(name);
}

/// <summary>What the shim calls. An exception is logged and rethrown so the host stops the script.</summary>
public static class Host
{
    static Script? script;
    static Script Current => script ??= Guest.Create();

    static void Run(Action call)
    {
        try { call(); }
        catch (Exception error)
        {
            World.Log("C# script error: " + error);
            throw;
        }
    }

    public static void Start() => Run(() => Current.Start());
    public static void Tick(float dt) => Run(() => Current.Tick(dt));
    public static void Frame(float dt) => Run(() => Current.Frame(dt));
    public static void Ui(float dt) => Run(() => Current.Ui(dt));
    public static void Stop() => Run(() => Current.Stop());
    public static void Destroy() => Run(() => Current.Destroy());
    public static void OnEvent(uint kind, double n0, double n1, double n2, double n3)
        => Run(() => Current.OnEvent(kind, n0, n1, n2, n3));
}
