import React from 'react';
import ReactDOM from 'react-dom/client';
import { QueryReviewApp } from './components/query-review/QueryReviewApp';
import { hydrateSettingsFromDisk } from './lib/settings';
import './styles.css';

const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);

hydrateSettingsFromDisk().finally(() => {
  root.render(
    <React.StrictMode>
      <QueryReviewApp />
    </React.StrictMode>,
  );
});
