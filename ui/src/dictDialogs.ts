// State for the dict name popup. Creating goes through the scope the
// sidebar button named; renaming needs none, since the backend resolves
// wherever the dict lives.
import { reactive } from 'vue';

export const dictDialog = reactive({
  mode: null as 'create' | 'rename' | null,
  renameTarget: '',
  /** Which dict a freshly created one goes in. */
  scope: 'actor' as 'actor' | 'global',
});

export function openCreateDictDialog(scope: 'actor' | 'global'): void {
  dictDialog.mode = 'create';
  dictDialog.renameTarget = '';
  dictDialog.scope = scope;
}

export function openRenameDictDialog(name: string): void {
  dictDialog.mode = 'rename';
  dictDialog.renameTarget = name;
}

export function closeDictDialog(): void {
  dictDialog.mode = null;
}
