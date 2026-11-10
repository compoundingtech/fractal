import { requireOptionalNativeModule } from 'expo';

const native = requireOptionalNativeModule<{ scan(): Promise<string | null> }>('FractalPairingScanner');
export const scanPairingQrNative = (): Promise<string | null> => {
  if (!native) return Promise.reject(new Error('QR scanning needs a native Fractal build. Enter pairing details manually.'));
  return native.scan();
};
export const isPairingQrPermissionDenied = (error: unknown): boolean =>
  !!error && typeof error === 'object' && 'code' in error && error.code === 'ERR_CAMERA_PERMISSION_DENIED';
