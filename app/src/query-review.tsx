import React from 'react';
import ReactDOM from 'react-dom/client';
import { QueryReviewApp } from './components/query-review/QueryReviewApp';
import { hydrateSettingsFromDisk } from './lib/settings';
import './styles.css';
import { useQueryAppearance } from './lib/hooks/useQueryAppearance';

function QueryReviewWindow() {
  useQueryAppearance();
  return <QueryReviewApp />;
}

const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);

hydrateSettingsFromDisk().finally(() => {
  root.render(
    <React.StrictMode>
      <QueryReviewWindow />
    </React.StrictMode>,
  );
});
