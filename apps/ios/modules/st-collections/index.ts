import { requireOptionalNativeModule } from 'expo';
import { collectionSocketFactory, type SocketBridge } from '../../collectionSocket';

const native = requireOptionalNativeModule<SocketBridge>('StCollections');
// Shipping iOS builds link this module. Expo Go cannot observe protocol pings.
export const nativeCollectionSocket = native ? collectionSocketFactory(native) : undefined;
