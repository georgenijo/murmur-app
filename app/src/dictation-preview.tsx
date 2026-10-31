import ReactDOM from 'react-dom/client';
import { mountDictationPreview } from './components/dictation-preview/bootstrap';
import './styles.css';

const root = ReactDOM.createRoot(document.getElementById('root') as HTMLElement);
void mountDictationPreview(root);
