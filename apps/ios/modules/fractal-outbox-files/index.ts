import { requireOptionalNativeModule } from 'expo';
import type { OutboxFilesBridge } from '../../outboxSpool.ts';

// Missing from an older installed binary means unprotected-storage, never memory fallback.
export default requireOptionalNativeModule<OutboxFilesBridge>('FractalOutboxFiles');
