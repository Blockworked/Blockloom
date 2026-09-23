<script setup lang="ts">
// Editing an actor's Rust script without leaving Blockloom. The file on disk
// is the document here - it isn't part of the project JSON - so this reads it
// on open and writes it on save. "Check" hands it to rustc and puts the
// result in the run log, and diagnostics come back pinned to their lines so
// they show inline too. The backend also keeps a Cargo project at the project
// root in sync, so an external editor gets the full rust-analyzer experience.
import { computed, onMounted, ref } from 'vue';
import {
  checkScript,
  openScriptIde,
  readScript,
  scriptDiagnostics,
  scriptToolchain,
  syncScriptIde,
  writeScript,
  type ScriptDiagnostic,
  type ToolchainStatus,
} from '../tauri';

const props = defineProps<{ actorId: string; actorName: string; path: string }>();
const emit = defineEmits<{ close: [] }>();

const source = ref('');
const error = ref('');
const busy = ref(true);
const diagnostics = ref<ScriptDiagnostic[]>([]);
const toolchain = ref<ToolchainStatus | null>(null);
const ideNote = ref('');
const area = ref<HTMLTextAreaElement | null>(null);
const gutter = ref<HTMLDivElement | null>(null);
const highlight = ref<HTMLPreElement | null>(null);

const lines = computed(() => source.value.split('\n').length);
const errorLines = computed(() => {
  const lines = new Set<number>();
  for (const d of diagnostics.value) if (d.level === 'error') lines.add(d.line);
  return lines;
});
const warningLines = computed(() => {
  const lines = new Set<number>();
  for (const d of diagnostics.value) if (d.level !== 'error') lines.add(d.line);
  return lines;
});

onMounted(async () => {
  try {
    source.value = await readScript(props.actorId);
  } catch (e) {
    error.value = String(e);
  }
  busy.value = false;
  area.value?.focus();
  void refreshDiagnostics();
  try {
    toolchain.value = await scriptToolchain();
  } catch (e) {
    error.value = String(e);
  }
});

async function refreshDiagnostics() {
  try {
    diagnostics.value = await scriptDiagnostics(props.actorId);
  } catch {
    diagnostics.value = [];
  }
}

async function save(): Promise<boolean> {
  error.value = '';
  try {
    await writeScript(props.actorId, source.value);
    return true;
  } catch (e) {
    error.value = String(e);
    return false;
  }
}

/** Saving first, so rustc is told about what's on screen rather than what was
 * last written. Diagnostics refresh after, so the gutter shows this edit. */
async function check() {
  busy.value = true;
  if (await save()) {
    try {
      await checkScript(props.actorId);
      await refreshDiagnostics();
    } catch (e) {
      error.value = String(e);
    }
  }
  busy.value = false;
}

async function saveAndClose() {
  if (await save()) emit('close');
}

/** Regenerates the Cargo project and points an external editor at it: VS Code
 * or Zed when one answers, else the folder in the file manager. Either way
 * rust-analyzer finds the root Cargo.toml from there. */
async function openExternal() {
  busy.value = true;
  ideNote.value = '';
  try {
    await syncScriptIde();
    const opened = await openScriptIde();
    ideNote.value = `Opened with ${opened.openedWith}: ${opened.path}`;
  } catch (e) {
    error.value = String(e);
  }
  busy.value = false;
}

function jumpTo(line: number, column: number) {
  const box = area.value;
  if (!box) return;
  const rows = source.value.split('\n');
  let offset = 0;
  for (let i = 0; i < line - 1 && i < rows.length; i++) offset += rows[i].length + 1;
  offset += Math.max(0, column - 1);
  box.focus();
  box.setSelectionRange(offset, offset);
  syncScroll();
}

function syncScroll() {
  const box = area.value;
  if (!box) return;
  if (highlight.value) {
    highlight.value.scrollTop = box.scrollTop;
    highlight.value.scrollLeft = box.scrollLeft;
  }
  if (gutter.value) gutter.value.scrollTop = box.scrollTop;
}

/** A code editor that swallows Tab is worse than one that doesn't indent, so
 * Tab inserts two spaces and stays in the box. */
function onTab(e: KeyboardEvent) {
  e.preventDefault();
  const box = e.target as HTMLTextAreaElement;
  const { selectionStart: from, selectionEnd: to } = box;
  source.value = `${source.value.slice(0, from)}  ${source.value.slice(to)}`;
  requestAnimationFrame(() => {
    box.setSelectionRange(from + 2, from + 2);
    syncScroll();
  });
}

// ─── Rust highlighting ──────────────────────────────────────────────────────
// A small tokenizer, not a grammar: keywords, types, strings, comments,
// numbers, macros, attributes and calls get their own class, and the overlay
// <pre> paints them under the transparent textarea. Good enough to read real
// Rust by, with no dependency to ship.

const KEYWORDS = new Set(
  'as,break,const,continue,crate,else,enum,extern,false,fn,for,if,impl,in,let,loop,match,mod,move,mut,pub,ref,return,self,Self,static,struct,super,trait,true,type,unsafe,use,where,while,async,await,dyn,try'.split(
    ',',
  ),
);

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

function highlightRust(src: string): string {
  let out = '';
  let i = 0;
  const n = src.length;
  const wrap = (cls: string, text: string) => `<span class="tok-${cls}">${escapeHtml(text)}</span>`;
  while (i < n) {
    const rest = src.slice(i);
    // Line comment.
    if (rest.startsWith('//')) {
      const end = src.indexOf('\n', i);
      const stop = end === -1 ? n : end;
      out += wrap('comment', src.slice(i, stop));
      i = stop;
      continue;
    }
    // Block comment (not nested - good enough for an editor tint).
    if (rest.startsWith('/*')) {
      const end = src.indexOf('*/', i + 2);
      const stop = end === -1 ? n : end + 2;
      out += wrap('comment', src.slice(i, stop));
      i = stop;
      continue;
    }
    // String or char literal.
    if (rest[0] === '"' || rest[0] === "'") {
      const quote = rest[0];
      let j = i + 1;
      while (j < n) {
        if (src[j] === '\\') j += 2;
        else if (src[j] === quote) {
          j += 1;
          break;
        } else if (src[j] === '\n' && quote === "'") break;
        else j += 1;
      }
      // A lone 'x' is a char; a longer '...' is lifetimes - leave those plain.
      const text = src.slice(i, j);
      out += text.length <= 4 || quote === '"' ? wrap('string', text) : escapeHtml(text);
      i = j;
      continue;
    }
    // Attribute.
    if (rest.startsWith('#')) {
      const end = src.indexOf(']', i);
      const stop = rest.startsWith('#[') && end !== -1 ? end + 1 : i + 1;
      out += wrap('attr', src.slice(i, stop));
      i = stop;
      continue;
    }
    // Number.
    const number = /^[0-9][0-9_]*(\.[0-9_]+)?(e[+-]?[0-9_]+)?(f32|f64|i32|u32|u64|usize)?/.exec(rest);
    if (number) {
      out += wrap('number', number[0]);
      i += number[0].length;
      continue;
    }
    // Word: keyword, macro call, call, type, or plain.
    const word = /^[A-Za-z_][A-Za-z0-9_]*/.exec(rest);
    if (word) {
      const text = word[0];
      const after = src.slice(i + text.length);
      if (KEYWORDS.has(text)) out += wrap('keyword', text);
      else if (after.startsWith('!')) out += wrap('macro', text) + '!';
      else if (after.startsWith('(')) out += wrap('call', text);
      else if (/^[A-Z]/.test(text)) out += wrap('type', text);
      else out += escapeHtml(text);
      i += text.length + (after.startsWith('!') ? 1 : 0);
      continue;
    }
    out += escapeHtml(rest[0]);
    i += 1;
  }
  return out + '\n';
}

const highlighted = computed(() => highlightRust(source.value));
</script>

<template>
  <div class="dialog-backdrop" @mousedown.self="emit('close')">
    <div class="dialog script-dialog">
      <h2>{{ actorName }} &middot; {{ path }}</h2>
      <p v-if="error" class="dialog-error">{{ error }}</p>
      <p v-if="toolchain && !toolchain.available" class="script-toolchain-missing">
        No Rust toolchain found - scripts won't run until one is installed. Blocks still run.
        {{ toolchain.help }}
      </p>
      <div class="script-editor">
        <div ref="gutter" class="script-gutter" aria-hidden="true">
          <div
            v-for="line in lines"
            :key="line"
            class="script-gutter-line"
            :class="{ error: errorLines.has(line), warning: !errorLines.has(line) && warningLines.has(line) }"
          >
            {{ line }}
          </div>
        </div>
        <div class="script-box">
          <pre ref="highlight" class="script-highlight" aria-hidden="true" v-html="highlighted" />
          <textarea
            ref="area"
            class="script-source script-input"
            spellcheck="false"
            v-model="source"
            @keydown.tab="onTab"
            @scroll="syncScroll"
            @input="syncScroll"
          />
        </div>
      </div>
      <ul v-if="diagnostics.length" class="script-diagnostics">
        <li
          v-for="(d, index) in diagnostics"
          :key="index"
          :class="d.level === 'error' ? 'diagnostic-error' : 'diagnostic-warning'"
        >
          <button class="diagnostic-jump" @click="jumpTo(d.line, d.column)">
            {{ d.line }}:{{ d.column }}
          </button>
          <span>{{ d.message }}</span>
        </li>
      </ul>
      <p class="panel-note">
        Real Rust, compiled with rustc when you press Play. `std` is there; other crates aren't.
        Errors land in the run log and on their lines above. The project root holds a Cargo.toml
        for rust-analyzer, so this file also opens in VS Code, Zed or RustRover with completion
        and go-to-source on the API.
      </p>
      <p v-if="ideNote" class="panel-note">{{ ideNote }}</p>
      <div class="dialog-actions">
        <button class="btn" @click="emit('close')">Cancel</button>
        <button class="btn" :disabled="busy" @click="openExternal">Open in editor</button>
        <button class="btn" :disabled="busy" @click="check">Check</button>
        <button class="btn primary" :disabled="busy" @click="saveAndClose">Save</button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.script-editor {
  display: flex;
  min-height: 0;
  border: 1px solid var(--border, #3a3f4b);
  border-radius: 6px;
  overflow: hidden;
}
.script-gutter {
  padding: 8px 6px 8px 10px;
  text-align: right;
  user-select: none;
  overflow: hidden;
  background: var(--panel-alt, #1c1f26);
  color: var(--muted, #8b93a3);
  font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
}
.script-gutter-line.error {
  color: #ff6b6b;
  font-weight: bold;
}
.script-gutter-line.warning {
  color: #e5c07b;
}
.script-box {
  position: relative;
  flex: 1;
  min-width: 0;
}
.script-highlight,
.script-input {
  margin: 0;
  padding: 8px 10px;
  font: 12px/1.5 ui-monospace, SFMono-Regular, Menlo, monospace;
  tab-size: 2;
  white-space: pre;
  overflow: auto;
}
.script-highlight {
  position: absolute;
  inset: 0;
  pointer-events: none;
}
.script-input {
  position: relative;
  width: 100%;
  height: 320px;
  box-sizing: border-box;
  border: none;
  resize: vertical;
  background: transparent;
  color: transparent;
  caret-color: var(--text, #eee);
}
.script-input::selection {
  background: rgb(76 151 255 / 0.35);
}
.script-diagnostics {
  list-style: none;
  margin: 8px 0 0;
  padding: 0;
  max-height: 120px;
  overflow: auto;
}
.diagnostic-error {
  color: #ff8080;
}
.diagnostic-warning {
  color: #e5c07b;
}
.diagnostic-jump {
  margin-right: 8px;
  text-decoration: underline;
  cursor: pointer;
}
.script-toolchain-missing {
  color: #ffb86b;
}
</style>

<style>
.tok-keyword {
  color: #c678dd;
}
.tok-string {
  color: #98c379;
}
.tok-comment {
  color: #7f848e;
  font-style: italic;
}
.tok-number {
  color: #d19a66;
}
.tok-type {
  color: #e5c07b;
}
.tok-call {
  color: #61afef;
}
.tok-macro {
  color: #56b6c2;
}
.tok-attr {
  color: #56b6c2;
}
</style>
