import { StyleSheet, View } from 'react-native';
import { theme } from './theme';
import { Button, T, useTextAppearance } from './ui';

/** The app-internal view of an active client-control grant; not a wire type. */
export type ClientControlGrant = { runId: string };

/**
 * Shown while a run may drive this client: what is controlling it, and how to stop it.
 * Renders nothing without a grant. Test-mode only until a generated grant source exists.
 */
export function ClientControlIndicator({ grant, onRevoke }: { grant: ClientControlGrant | undefined; onRevoke: () => void }) {
  const appearance = useTextAppearance();
  if (!grant) return null;
  return (
    <View
      testID="client-control-indicator"
      style={[styles.bar, { borderLeftColor: appearance.color(theme.working), backgroundColor: appearance.system ? appearance.colors.grouped : appearance.palette.mantle }]}
    >
      <View
        style={styles.label}
        accessible
        accessibilityRole="text"
        accessibilityLabel={`Remote control active. Run ${grant.runId} can operate this app.`}
      >
        <T bold color={theme.working}>Controlled by run</T>
        <T dim numberOfLines={1}>{grant.runId}</T>
      </View>
      <Button label="Revoke" color={theme.fault} onPress={onRevoke} style={styles.revoke} />
    </View>
  );
}

const styles = StyleSheet.create({
  bar: { flexDirection: 'row', alignItems: 'center', gap: 12, borderLeftWidth: 3, paddingHorizontal: 12, paddingVertical: 8 },
  label: { flex: 1, minWidth: 0 },
  revoke: { marginTop: 0, alignSelf: 'center' },
});
