// State for the variable name popup.
import { reactive } from 'vue';

export const variableDialog = reactive({
  mode: null as 'create' | 'rename' | null,
  renameTarget: '',
  /** Which list a freshly created variable goes in. */
  scope: 'actor' as 'actor' | 'global',
});

export function openCreateVariableDialog(scope: 'actor' | 'global'): void {
  variableDialog.mode = 'create';
  variableDialog.renameTarget = '';
  variableDialog.scope = scope;
}

export function openRenameVariableDialog(name: string): void {
  variableDialog.mode = 'rename';
  variableDialog.renameTarget = name;
}

export function closeVariableDialog(): void {
  variableDialog.mode = null;
}
