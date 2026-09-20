import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App';
import './styles.css';
import { applyTheme, readThemePreference } from './lib/theme';

// Set the palette before React renders the first frame.
applyTheme(readThemePreference());

ReactDOM.createRoot(document.getElementById('root')!).render(<React.StrictMode><App /></React.StrictMode>);
