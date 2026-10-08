import { createRoot } from 'react-dom/client';
import { App } from './App';
import { RecallService, nativeTransport } from './service/client';
import './styles.css';
const transport = nativeTransport();
createRoot(document.getElementById('root')!).render(<App service={transport ? new RecallService(transport) : null} />);
