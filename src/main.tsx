import './index.css';
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import Overlay from './Overlay';

// Route: render the transparent overlay if URL hash or search param indicates overlay
const isOverlay =
  window.location.hash.includes('overlay') ||
  window.location.search.includes('overlay=1') ||
  window.location.pathname.includes('overlay');

// Apply transparent body/html class so the Tauri transparent window shows through
if (isOverlay) {
  document.documentElement.classList.add('overlay-active');
  document.body.classList.add('overlay-active');
}

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    {isOverlay ? <Overlay /> : <App />}
  </React.StrictMode>,
);
