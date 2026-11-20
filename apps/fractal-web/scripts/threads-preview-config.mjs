/** Derive the preview's isolated instance, daemon unit and default state root without I/O. */
export const derivePreviewConfig = ({ PREVIEW_INSTANCE = 'threadspreview', PREVIEW_ROOT } = {}) => {
  // Mirror crates/fractal-kernel/src/instance.rs::validate before creating preview state.
  if (typeof PREVIEW_INSTANCE !== 'string' || PREVIEW_INSTANCE.length === 0 || PREVIEW_INSTANCE.length > 24) {
    throw new Error('PREVIEW_INSTANCE must be 1 to 24 characters')
  }
  if (/[^a-z0-9-]/.test(PREVIEW_INSTANCE)) {
    throw new Error('PREVIEW_INSTANCE must use lowercase letters, digits and hyphens')
  }
  if (PREVIEW_INSTANCE.startsWith('-') || PREVIEW_INSTANCE.endsWith('-') || PREVIEW_INSTANCE.includes('--')) {
    throw new Error('PREVIEW_INSTANCE must not start or end with a hyphen or repeat one')
  }
  return {
    instance: PREVIEW_INSTANCE,
    unit: `${PREVIEW_INSTANCE}-fractal-up`,
    root: PREVIEW_ROOT ?? `/var/tmp/fractal-${PREVIEW_INSTANCE}`,
  }
}
