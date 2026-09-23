<script setup lang="ts">
// What each right-click menu offers. The panel itself (placing, closing,
// drawing the list) comes from blockstitch; this is only the content.
import { computed } from 'vue';
import {
  ContextMenuPanel,
  focusCommentOnMount,
  nextSiblingPath,
  regenerateInstructionIds,
  resolveInstructionAt,
  type ContextMenuItem,
} from 'blockstitch';
import {
  ClipboardPaste,
  Copy,
  Layers,
  MessageSquare,
  Pencil,
  Scissors,
  Trash2,
} from 'lucide-vue-next';
import { openActor } from '../store';
import { closeContextMenu, contextMenu } from '../contextMenu';
import { clipboardContents, copyBlock, copyStack, hasClipboard } from '../clipboard';
import {
  addInstruction,
  createAttachedComment,
  createComment,
  deleteBlock,
  deleteInstruction,
  deleteList,
  deleteVariable,
  pasteInstructions,
  setCommentCollapsed,
} from '../tauri';
import { openRenameVariableDialog } from '../variableDialogs';
import { openRenameListDialog } from '../listDialogs';
import { forgetListEditor } from 'blockstitch';
import { openEditBlockDialog } from '../blockDialogs';
import { findBlockDef, type InstructionDto } from '../types';

/** Where a freshly attached note spawns, clear of the block it belongs to. */
const NOTE_OFFSET = { dx: 220, dy: 0 };

const strand = computed(
  () => openActor.value?.strands.find(candidate => candidate.id === contextMenu.strandId) ?? null,
);
const instruction = computed(() => resolveInstructionAt(strand.value, contextMenu.path) as InstructionDto | null);
const attachedNote = computed(() => {
  const id = instruction.value?.id;
  if (!id) return null;
  return openActor.value?.comments.find(comment => comment.attached_to === id) ?? null;
});

function focusNote(commentId: string) {
  focusCommentOnMount(commentId);
  void setCommentCollapsed(commentId, false);
}

const items = computed<ContextMenuItem[]>(() => {
  switch (contextMenu.type) {
    case 'block': {
      const current = strand.value;
      const target = instruction.value;
      if (!current || !target) return [];
      return [
        {
          key: 'duplicate',
          label: 'Duplicate',
          icon: Copy,
          onSelect: () =>
            void addInstruction(
              current.id,
              nextSiblingPath(contextMenu.path),
              regenerateInstructionIds(target) as InstructionDto,
            ),
        },
        { key: 'copy', label: 'Copy block', icon: Copy, onSelect: () => copyBlock(current, contextMenu.path) },
        {
          key: 'copy-stack',
          label: 'Copy from here down',
          icon: Layers,
          onSelect: () => copyStack(current, contextMenu.path),
        },
        {
          key: 'note',
          label: attachedNote.value ? 'Edit note' : 'Add a note',
          icon: MessageSquare,
          onSelect: () => {
            const existing = attachedNote.value;
            if (existing) {
              focusNote(existing.id);
              return;
            }
            void createAttachedComment(target.id, NOTE_OFFSET.dx, NOTE_OFFSET.dy, '').then(focusNote);
          },
        },
        {
          key: 'delete',
          label: 'Delete this block',
          icon: Trash2,
          danger: true,
          onSelect: () =>
            void deleteInstruction(current.id, contextMenu.path, current.x + 40, current.y + 40),
        },
      ];
    }
    case 'canvas':
      return [
        {
          key: 'paste',
          label: 'Paste blocks',
          icon: ClipboardPaste,
          disabled: !hasClipboard(),
          onSelect: () => {
            const copied = clipboardContents();
            if (!copied) return;
            void pasteInstructions(
              contextMenu.canvasX,
              contextMenu.canvasY,
              copied.map(block => regenerateInstructionIds(block) as InstructionDto),
            );
          },
        },
        {
          key: 'note',
          label: 'Add a note',
          icon: MessageSquare,
          onSelect: () =>
            void createComment(contextMenu.canvasX, contextMenu.canvasY, '').then(focusNote),
        },
      ];
    case 'variable':
      return [
        {
          key: 'rename',
          label: `Rename "${contextMenu.variableName}"`,
          icon: Pencil,
          onSelect: () => openRenameVariableDialog(contextMenu.variableName),
        },
        {
          key: 'delete',
          label: `Delete "${contextMenu.variableName}"`,
          icon: Trash2,
          danger: true,
          onSelect: () => void deleteVariable(contextMenu.variableName),
        },
      ];
    case 'list':
      return [
        {
          key: 'rename',
          label: `Rename "${contextMenu.listName}"`,
          icon: Pencil,
          onSelect: () => openRenameListDialog(contextMenu.listName),
        },
        {
          key: 'delete',
          label: `Delete "${contextMenu.listName}"`,
          icon: Trash2,
          danger: true,
          onSelect: () => {
            forgetListEditor(contextMenu.listName);
            void deleteList(contextMenu.listName);
          },
        },
      ];
    case 'myBlock': {
      const def = findBlockDef(openActor.value, contextMenu.blockId);
      if (!def) return [];
      return [
        { key: 'edit', label: 'Edit this block', icon: Pencil, onSelect: () => openEditBlockDialog(def) },
        {
          key: 'delete',
          label: 'Delete this block',
          icon: Scissors,
          danger: true,
          onSelect: () => void deleteBlock(def.id),
        },
      ];
    }
    // A sidebar prefab and a placed value have nothing to act on yet.
    default:
      return [];
  }
});
</script>

<template>
  <ContextMenuPanel
    :open="contextMenu.open && items.length > 0"
    :x="contextMenu.x"
    :y="contextMenu.y"
    :items="items"
    @close="closeContextMenu"
  />
</template>
