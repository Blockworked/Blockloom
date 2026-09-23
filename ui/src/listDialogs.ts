// State for the list name popup. Creating goes through the scope the
// sidebar button named; renaming needs none, since the backend resolves
// wherever the list lives.
import { reactive } from 'vue';

export const listDialog = reactive({
  mode: null as 'create' | 'rename' | null,
  renameTarget: '',
  /** Which list a freshly created one goes in. */
  scope: 'actor' as 'actor' | 'global',
});

export function openCreateListDialog(scope: 'actor' | 'global'): void {
  listDialog.mode = 'create';
  listDialog.renameTarget = '';
  listDialog.scope = scope;
}

export function openRenameListDialog(name: string): void {
  listDialog.mode = 'rename';
  listDialog.renameTarget = name;
}

export function closeListDialog(): void {
  listDialog.mode = null;
}
