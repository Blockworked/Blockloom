import 'blockstitch/theme.css';
import './style.css';
import { createApp, watch } from 'vue';
import { useTheme } from 'blockstitch';
import App from './App.vue';
import { setupBlockstitch } from './blockstitchSetup';
import { setThemeBackground } from './tauri';

setupBlockstitch();
// Keep the native window's background matched to the theme, so nothing flashes
// white before the page paints or while the window closes.
watch(useTheme().currentTheme, theme => void setThemeBackground(theme).catch(() => {}), { immediate: true });
createApp(App).mount('#app');
