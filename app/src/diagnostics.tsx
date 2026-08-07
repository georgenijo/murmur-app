import React from 'react';
import ReactDOM from 'react-dom/client';
import { DiagnosticsWindowApp } from './components/log-viewer/DiagnosticsWindowApp';
import './styles.css';

ReactDOM.createRoot(document.getElementById('root') as HTMLElement).render(
  <React.StrictMode>
    <DiagnosticsWindowApp />
  </React.StrictMode>,
);
