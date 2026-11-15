/** Derive the preview's isolated instance, daemon unit and default state root without I/O. */
export const derivePreviewConfig = ({ PREVIEW_INSTANCE = 'threadspreview', PREVIEW_ROOT } = {}) => {
  if (typeof PREVIEW_INSTANCE !== 'string' || PREVIEW_INSTANCE.length === 0 || /[^a-z0-9]/.test(PREVIEW_INSTANCE)) {
    throw new Error('PREVIEW_INSTANCE must contain only lowercase letters and digits (a-z, 0-9), and must not be empty')
  }
  return {
    instance: PREVIEW_INSTANCE,
    unit: `${PREVIEW_INSTANCE}-fractal-up`,
    root: PREVIEW_ROOT ?? `/var/tmp/fractal-${PREVIEW_INSTANCE}`,
  }
}
