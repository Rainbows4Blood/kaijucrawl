// Development-only page, excluded from the single-entry production build.
import { createApp } from '../app';
import { previewInvoke } from './fixture';
import '../style.css';
createApp(document.querySelector<HTMLDivElement>('#app')!, previewInvoke);
