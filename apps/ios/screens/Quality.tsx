import { useCallback, useState } from 'react';
import { useFocusEffect } from '@react-navigation/native';
import { View } from 'react-native';
import { quality } from '../qualityNative';
import { qualityRows } from '../qualityView';
import { SectionHeader, T } from '../ui';
import { theme } from '../theme';

/** Reactive local history; no gateway read and no polling while Fleet is open. */
export function QualitySection() {
  const [report, setReport] = useState(() => quality.report());
  useFocusEffect(useCallback(() => {
    const update = () => setReport(quality.report());
    update();
    return quality.subscribe(update);
  }, []));
  const rows = qualityRows(report.last, report.day);
  const color = (pass: boolean | null) => pass === null ? theme.overlay0 : pass ? theme.green : theme.fault;
  return <>
    <SectionHeader title="quality" />
    <View style={{ paddingHorizontal: 12, gap: 4 }}>
      <T dim>This device only · initial targets</T>
      {rows.length ? <>
        <View style={{ flexDirection: 'row' }}><T dim style={{ flex: 1 }}>measure · target</T><T dim style={{ width: 72 }}>last hour</T><T dim style={{ width: 72 }}>24 hours</T></View>
        {rows.map(row => <View key={row.key} style={{ flexDirection: 'row' }}>
          <T style={{ flex: 1, fontSize: 11 }}>{row.label} · {row.target}</T>
          <T color={color(row.hourPass)} style={{ width: 72, fontSize: 12 }}>{row.hour}</T>
          <T color={color(row.dayPass)} style={{ width: 72, fontSize: 12 }}>{row.day}</T>
        </View>)}
        <T dim>Green meets target · red misses · — no sample</T>
        <T dim>{report.day?.counters.drafts_restored_unclean ?? 0} drafts restored after an unclean exit · {Math.round((report.day?.counters.not_live_ms ?? 0) / 1000)}s not live in 24h</T>
        <T dim>{report.day?.counters.gateway_attach ?? 0} gateway / fallback attaches · {report.day?.counters.dead_terminal_tabs ?? 0} interrupted terminal tabs</T>
      </> : <T dim>No completed hour yet. Hours are kept locally for fourteen days.</T>}
    </View>
  </>;
}
