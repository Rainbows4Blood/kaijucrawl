import { invoke } from '@tauri-apps/api/core';
import { createApp } from './app';
import './style.css';
createApp(document.querySelector<HTMLDivElement>('#app')!, invoke);
