// The frontend's one copy of backend state, replaced wholesale after commands
// and on runtime state events. Nothing here is edited locally.
import { computed, reactive, ref } from 'vue';
import { configureDictEditorPersistence, configureListEditorPersistence } from 'blockstitch';
import { emptyState, findActor, type StateDto } from './types';
import { getAppVersion, getState, onStateUpdated, setDictEditorState, setListEditorState } from './tauri';

configureListEditorPersistence((name, visible, x, y) => {
  void setListEditorState(name, visible, x, y).catch(error => {
    console.error('Failed to save list editor state:', error);
  });
});

configureDictEditorPersistence((name, visible, x, y) => {
  void setDictEditorState(name, visible, x, y).catch(error => {
    console.error('Failed to save dict editor state:', error);
  });
});

export const state = reactive<StateDto>(emptyState());
export const appVersion = ref('');

/** The actor whose canvas is open. */
export const openActor = computed(() => findActor(state.project, state.selected_actor));

/** The project's dimension - what the palette and the inspector key off. */
export const mode = computed(() => state.project?.world.mode ?? 'TwoD');

let initialized = false;

export async function initState(): Promise<void> {
  if (initialized) return;
  initialized = true;

  try {
    appVersion.value = await getAppVersion();
  } catch (e) {
    console.error('Failed to get the app version:', e);
  }
  try {
    Object.assign(state, await getState());
  } catch (e) {
    console.error('Failed to get the initial state:', e);
  }
  try {
    await onStateUpdated(next => Object.assign(state, next));
  } catch (e) {
    console.error('Failed to subscribe to state updates:', e);
  }
}
