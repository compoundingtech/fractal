import { useCallback } from 'react';
import { Linking, ScrollView, View } from 'react-native';
import { useFocusEffect } from '@react-navigation/native';
import type { RootScreen } from '../navigation';
import { Banners, StatusLine } from '../chrome';
import { resourceFacts, resourceRow } from '../sidebarResources';
import { useStore } from '../store';
import { Button, Note, Screen, T } from '../ui';

/** Open vocabulary facts are displayed as observed, without inventing a resource-specific schema. */
export function ResourceScreen({ route }: RootScreen<'Resource'>) {
  const { sidebarResources, sidebarResourcesIssue, loadSidebarResources, status } = useStore();
  useFocusEffect(useCallback(() => { if (status === 'online') void loadSidebarResources(route.params.id); }, [status, loadSidebarResources, route.params.id]));
  const row = sidebarResources.find(row => row.id === route.params.id), facts = row ? resourceFacts(row) : {};
  const url = typeof facts.url === 'string' && /^https?:\/\//.test(facts.url) ? facts.url : undefined;
  return <Screen><Banners /><ScrollView contentInsetAdjustmentBehavior="automatic"><StatusLine />
    {sidebarResourcesIssue ? <Note tone="warning">{sidebarResourcesIssue}</Note> : null}
    <View style={{ padding: 12 }}><T bold>{row ? resourceRow(row).title : route.params.title || route.params.id}</T><T dim>{route.params.id}</T>
      {row ? <><Note>{row.kind} · observed {row.observed_at}</Note>
        {url ? <Button label="Open link" onPress={() => void Linking.openURL(url)} /> : null}
        {Object.entries(facts).map(([name, value]) => <View key={name} style={{ paddingVertical: 8 }}><T bold>{name}</T><T>{typeof value === 'string' ? value : JSON.stringify(value)}</T></View>)}
      </> : <Note>Resource unavailable in the current window.</Note>}
    </View>
  </ScrollView></Screen>;
}
