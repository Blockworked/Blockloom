// State for the "Make a Block" / "Edit Block" popup.
import { reactive } from 'vue';
import type { BlockDefDto } from './types';

export const blockDialog = reactive({
  mode: null as 'create' | 'edit' | null,
  editTarget: null as BlockDefDto | null,
});

export function openCreateBlockDialog(): void {
  blockDialog.mode = 'create';
  blockDialog.editTarget = null;
}

export function openEditBlockDialog(def: BlockDefDto): void {
  blockDialog.mode = 'edit';
  blockDialog.editTarget = def;
}

export function closeBlockDialog(): void {
  blockDialog.mode = null;
  blockDialog.editTarget = null;
}
