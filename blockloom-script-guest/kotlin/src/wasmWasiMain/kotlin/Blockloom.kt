// The Kotlin half of the guest: typed access to the world's imports and the
// dispatcher the exports call. Strings cross the component boundary as UTF-8
// in linear memory, which Kotlin/Wasm leaves entirely to this code.
@file:OptIn(
    kotlin.wasm.ExperimentalWasmInterop::class,
    kotlin.wasm.unsafe.UnsafeWasmMemoryApi::class,
    kotlin.wasm.unsafe.ComponentModelInternalApi::class,
)

import kotlin.wasm.WasmExport
import kotlin.wasm.WasmImport
import kotlin.wasm.unsafe.MemoryAllocator
import kotlin.wasm.unsafe.Pointer
import kotlin.wasm.unsafe.componentModelRealloc
import kotlin.wasm.unsafe.freeAllComponentModelReallocAllocatedMemory
import kotlin.wasm.unsafe.withScopedMemoryAllocator

/** Override what you need; every call is one entry point of the world. */
abstract class Script {
    open fun start() {}
    open fun tick(dt: Float) {}
    open fun frame(dt: Float) {}
    open fun ui(dt: Float) {}
    open fun stop() {}
    open fun destroy() {}
    /** [kind] is an abi `EVENT_*` number. */
    open fun onEvent(kind: Int, n0: Double, n1: Double, n2: Double, n3: Double) {}
}

/** Raw verbs, numbered as blockloom-core/src/script/abi.rs. */
object Verbs {
    const val READ_POSITION = 1
    const val ACT_MOVE = 1
    const val ACT_GO_TO = 2
    const val ACT_CHANGE_POSITION = 3
    const val ACT_SAY = 10
    const val ACT_LOG = 19
}

// Lowered imports, as the canonical ABI flattens them: a string is (ptr, len)
// and a result too big to return comes back through a trailing pointer.
@WasmImport("blockloom:script/sensors@0.2.0", "read")
private external fun importRead(what: Int, aPtr: Int, aLen: Int, bPtr: Int, bLen: Int, arg: Double, ret: Int)
@WasmImport("blockloom:script/texts@0.2.0", "read-text")
private external fun importReadText(what: Int, aPtr: Int, aLen: Int, bPtr: Int, bLen: Int, ret: Int)
@WasmImport("blockloom:script/acts@0.2.0", "act")
private external fun importAct(what: Int, aPtr: Int, aLen: Int, bPtr: Int, bLen: Int, cPtr: Int, cLen: Int, nPtr: Int, nLen: Int)
@WasmImport("blockloom:script/state@0.2.0", "get-number")
private external fun importStateGetNumber(kPtr: Int, kLen: Int, ret: Int)
@WasmImport("blockloom:script/state@0.2.0", "get-text")
private external fun importStateGetText(kPtr: Int, kLen: Int, ret: Int)
@WasmImport("blockloom:script/state@0.2.0", "set-number")
private external fun importStateSetNumber(kPtr: Int, kLen: Int, value: Double)
@WasmImport("blockloom:script/state@0.2.0", "set-text")
private external fun importStateSetText(kPtr: Int, kLen: Int, vPtr: Int, vLen: Int)
@WasmImport("blockloom:script/state@0.2.0", "clear")
private external fun importStateClear(kPtr: Int, kLen: Int)
@WasmImport("blockloom:script/vars@0.2.0", "get-number")
private external fun importVarGetNumber(nPtr: Int, nLen: Int): Double
@WasmImport("blockloom:script/vars@0.2.0", "get-text")
private external fun importVarGetText(nPtr: Int, nLen: Int, ret: Int)
@WasmImport("blockloom:script/vars@0.2.0", "set-number")
private external fun importVarSetNumber(nPtr: Int, nLen: Int, value: Double)
@WasmImport("blockloom:script/vars@0.2.0", "set-text")
private external fun importVarSetText(nPtr: Int, nLen: Int, vPtr: Int, vLen: Int)
@WasmImport("blockloom:script/lists@0.2.0", "len")
private external fun importListLen(nPtr: Int, nLen: Int): Int
@WasmImport("blockloom:script/lists@0.2.0", "get-number")
private external fun importListGetNumber(nPtr: Int, nLen: Int, index: Int, ret: Int)
@WasmImport("blockloom:script/lists@0.2.0", "get-text")
private external fun importListGetText(nPtr: Int, nLen: Int, index: Int, ret: Int)
@WasmImport("blockloom:script/lists@0.2.0", "add-number")
private external fun importListAddNumber(nPtr: Int, nLen: Int, value: Double)
@WasmImport("blockloom:script/lists@0.2.0", "add-text")
private external fun importListAddText(nPtr: Int, nLen: Int, vPtr: Int, vLen: Int)
@WasmImport("blockloom:script/lists@0.2.0", "insert-number")
private external fun importListInsertNumber(nPtr: Int, nLen: Int, index: Int, value: Double)
@WasmImport("blockloom:script/lists@0.2.0", "insert-text")
private external fun importListInsertText(nPtr: Int, nLen: Int, index: Int, vPtr: Int, vLen: Int)
@WasmImport("blockloom:script/lists@0.2.0", "replace-number")
private external fun importListReplaceNumber(nPtr: Int, nLen: Int, index: Int, value: Double)
@WasmImport("blockloom:script/lists@0.2.0", "replace-text")
private external fun importListReplaceText(nPtr: Int, nLen: Int, index: Int, vPtr: Int, vLen: Int)
@WasmImport("blockloom:script/lists@0.2.0", "delete")
private external fun importListDelete(nPtr: Int, nLen: Int, index: Int)
@WasmImport("blockloom:script/lists@0.2.0", "clear")
private external fun importListClear(nPtr: Int, nLen: Int)

// What the host's `cabi_realloc` needs to place a string it hands back.
@WasmExport("cabi_realloc")
fun cabiRealloc(oldPtr: Int, oldSize: Int, align: Int, newSize: Int): Int =
    componentModelRealloc(oldPtr, oldSize, newSize)

private fun at(address: Int, offset: Int = 0) = Pointer((address + offset).toUInt())

/** A UTF-8 copy of [text] in linear memory: (address, length). */
private class Utf8(val ptr: Int, val len: Int)

private fun MemoryAllocator.utf8(text: String): Utf8 {
    val bytes = text.encodeToByteArray()
    val ptr = allocate(bytes.size.coerceAtLeast(1)).address.toInt()
    for (i in bytes.indices) at(ptr, i).storeByte(bytes[i])
    return Utf8(ptr, bytes.size)
}

private fun MemoryAllocator.doubles(numbers: DoubleArray): Utf8 {
    val ptr = allocate((numbers.size * 8).coerceAtLeast(8)).address.toInt()
    for (i in numbers.indices) at(ptr, i * 8).storeLong(numbers[i].toBits())
    return Utf8(ptr, numbers.size)
}

private fun readString(ptr: Int, len: Int): String {
    val bytes = ByteArray(len)
    for (i in 0 until len) bytes[i] = at(ptr, i).loadByte()
    return bytes.decodeToString()
}

/** The string a host call left at [area] + [offset] (ptr, len), then its memory is freed. */
private fun takeString(area: Int, offset: Int): String {
    val text = readString(at(area, offset).loadInt(), at(area, offset + 4).loadInt())
    freeAllComponentModelReallocAllocatedMemory()
    return text
}

private fun MemoryAllocator.area(size: Int): Int = allocate(size).address.toInt()

/** Snapshot reads and world writes, with the verbs as plain numbers. */
object World {
    /** A number read by abi `READ_*` verb. Null when the world has no such thing. */
    fun read(what: Int, a: String = "", b: String = "", arg: Double = 0.0): Double? =
        withScopedMemoryAllocator { m ->
            val sa = m.utf8(a)
            val sb = m.utf8(b)
            val ret = m.area(16)
            importRead(what, sa.ptr, sa.len, sb.ptr, sb.len, arg, ret)
            if (at(ret).loadByte().toInt() == 0) Double.fromBits(at(ret, 8).loadLong()) else null
        }

    fun readText(what: Int, a: String = "", b: String = ""): String? =
        withScopedMemoryAllocator { m ->
            val sa = m.utf8(a)
            val sb = m.utf8(b)
            val ret = m.area(12)
            importReadText(what, sa.ptr, sa.len, sb.ptr, sb.len, ret)
            if (at(ret).loadByte().toInt() == 0) takeString(ret, 4) else null
        }

    fun act(what: Int, a: String = "", b: String = "", c: String = "", vararg numbers: Double) {
        withScopedMemoryAllocator { m ->
            val sa = m.utf8(a)
            val sb = m.utf8(b)
            val sc = m.utf8(c)
            val list = m.doubles(numbers)
            importAct(what, sa.ptr, sa.len, sb.ptr, sb.len, sc.ptr, sc.len, list.ptr, list.len)
        }
    }

    fun say(text: String) = act(Verbs.ACT_SAY, text)
    fun log(text: String) = act(Verbs.ACT_LOG, text)
}

/** Per-actor state that lasts the run. */
object State {
    fun number(key: String): Double? = withScopedMemoryAllocator { m ->
        val k = m.utf8(key)
        val ret = m.area(16)
        importStateGetNumber(k.ptr, k.len, ret)
        if (at(ret).loadByte().toInt() == 0) Double.fromBits(at(ret, 8).loadLong()) else null
    }

    fun text(key: String): String? = withScopedMemoryAllocator { m ->
        val k = m.utf8(key)
        val ret = m.area(12)
        importStateGetText(k.ptr, k.len, ret)
        if (at(ret).loadByte().toInt() == 0) takeString(ret, 4) else null
    }

    fun set(key: String, value: Double) {
        withScopedMemoryAllocator { m -> val k = m.utf8(key); importStateSetNumber(k.ptr, k.len, value) }
    }

    fun set(key: String, value: String) {
        withScopedMemoryAllocator { m -> val k = m.utf8(key); val v = m.utf8(value); importStateSetText(k.ptr, k.len, v.ptr, v.len) }
    }

    fun clear(key: String) {
        withScopedMemoryAllocator { m -> val k = m.utf8(key); importStateClear(k.ptr, k.len) }
    }
}

/** Project variables, by name. */
object Vars {
    fun number(name: String): Double = withScopedMemoryAllocator { m -> val n = m.utf8(name); importVarGetNumber(n.ptr, n.len) }

    fun text(name: String): String = withScopedMemoryAllocator { m ->
        val n = m.utf8(name)
        val ret = m.area(8)
        importVarGetText(n.ptr, n.len, ret)
        takeString(ret, 0)
    }

    fun set(name: String, value: Double) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); importVarSetNumber(n.ptr, n.len, value) }
    }

    fun set(name: String, value: String) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); val v = m.utf8(value); importVarSetText(n.ptr, n.len, v.ptr, v.len) }
    }
}

/** Project lists, by name. */
object Lists {
    fun count(name: String): Int = withScopedMemoryAllocator { m -> val n = m.utf8(name); importListLen(n.ptr, n.len) }

    fun number(name: String, index: Int): Double? = withScopedMemoryAllocator { m ->
        val n = m.utf8(name)
        val ret = m.area(16)
        importListGetNumber(n.ptr, n.len, index, ret)
        if (at(ret).loadByte().toInt() == 0) Double.fromBits(at(ret, 8).loadLong()) else null
    }

    fun text(name: String, index: Int): String? = withScopedMemoryAllocator { m ->
        val n = m.utf8(name)
        val ret = m.area(12)
        importListGetText(n.ptr, n.len, index, ret)
        if (at(ret).loadByte().toInt() == 0) takeString(ret, 4) else null
    }

    fun add(name: String, value: Double) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); importListAddNumber(n.ptr, n.len, value) }
    }

    fun add(name: String, value: String) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); val v = m.utf8(value); importListAddText(n.ptr, n.len, v.ptr, v.len) }
    }

    fun insert(name: String, index: Int, value: Double) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); importListInsertNumber(n.ptr, n.len, index, value) }
    }

    fun insert(name: String, index: Int, value: String) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); val v = m.utf8(value); importListInsertText(n.ptr, n.len, index, v.ptr, v.len) }
    }

    fun replace(name: String, index: Int, value: Double) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); importListReplaceNumber(n.ptr, n.len, index, value) }
    }

    fun replace(name: String, index: Int, value: String) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); val v = m.utf8(value); importListReplaceText(n.ptr, n.len, index, v.ptr, v.len) }
    }

    fun delete(name: String, index: Int) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); importListDelete(n.ptr, n.len, index) }
    }

    fun clear(name: String) {
        withScopedMemoryAllocator { m -> val n = m.utf8(name); importListClear(n.ptr, n.len) }
    }
}

private var script: Script? = null

// The guest defines `fun createScript(): Script`, in this same module.
private fun current(): Script = script ?: createScript().also { script = it }

// Exceptions are logged and rethrown, so the host stops the script.
private inline fun run(call: () -> Unit) {
    try {
        call()
    } catch (error: Throwable) {
        World.log("Kotlin script error: $error")
        throw error
    }
}

@WasmExport("blockloom:script/entry@0.2.0#start")
fun entryStart() = run { current().start() }

@WasmExport("blockloom:script/entry@0.2.0#tick")
fun entryTick(dt: Float) = run { current().tick(dt) }

@WasmExport("blockloom:script/entry@0.2.0#frame")
fun entryFrame(dt: Float) = run { current().frame(dt) }

@WasmExport("blockloom:script/entry@0.2.0#ui")
fun entryUi(dt: Float) = run { current().ui(dt) }

@WasmExport("blockloom:script/entry@0.2.0#stop")
fun entryStop() = run { current().stop() }

@WasmExport("blockloom:script/entry@0.2.0#destroy")
fun entryDestroy() = run { current().destroy() }

@WasmExport("blockloom:script/entry@0.2.0#on-event")
fun entryOnEvent(kind: Int, n0: Double, n1: Double, n2: Double, n3: Double) =
    run { current().onEvent(kind, n0, n1, n2, n3) }
