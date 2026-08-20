import React from 'react';
import ReactDOM from 'react-dom/client';
import { DictationPreviewApp } from './components/dictation-preview/DictationPreviewApp';
import './styles.css';

const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);

root.render(
  <React.StrictMode>
    <DictationPreviewApp />
  </React.StrictMode>,
);
