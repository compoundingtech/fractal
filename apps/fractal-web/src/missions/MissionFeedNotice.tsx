import { SystemNotice } from '@fractal/ui/assistant-ui/shell'
import type * as Fractal from '@fractal/client/schema'
import * as React from 'react'

import { credentialExpiredMessage, type Feed } from '../data/source.ts'

/** What the feed says while no row can be listed; null once there are rows to show. */
export const MissionFeedNotice = ({ missions, total, shown }: { readonly missions: Feed<readonly Fractal.Mission[]>; readonly total: number; readonly shown: number }): React.ReactNode =>
  missions._tag === 'Waiting' ? <SystemNotice kind="loading" title="Loading missions" />
    : missions._tag === 'Unavailable' ? <SystemNotice
      kind="error"
      connection={missions.reason !== 'ungranted' && missions.reason !== 'unsupported'}
      title={missions.reason === 'unauthorized' ? credentialExpiredMessage : missions.reason === 'ungranted' ? 'No access to missions' : missions.reason === 'unsupported' ? 'Missions not supported' : 'Missions could not load'}
      {...(missions.reason === 'ungranted' ? { detail: 'Read access has not been granted' } : missions.reason === 'unsupported' ? { detail: 'This connection does not offer them' } : {})}
    />
      : shown === 0 ? <SystemNotice kind="empty" title={total === 0 ? 'No missions' : 'Nothing running today'} />
        : null

/** The server bounds this feed by bytes and offers no continuation on a live subscription, so the total is unknown. */
export const missionsTruncatedText = (count: number): string => `First ${count} missions. More exist that this view cannot load yet.`
