/** App destinations, not device.command wire types. The transport must decode its generated screen first. */
export type ClientControlRouteTarget =
  | { readonly _tag: 'NowRoot' }
  | { readonly _tag: 'Attention'; readonly askId: string }

export interface ClientControlNavigation {
  /** Resolve through the app's subject model; no test-only renderer or fixtureSource. */
  readonly href: (target: ClientControlRouteTarget) => string
  readonly navigate: (href: string) => void
}

/**
 * The integration host supplies subjectPath/subjectHref from workspace/subject.ts. Keeping its
 * routing codec authoritative prevents the control transport growing a second URL convention.
 */
export const navigateClientControlRoute = (
  target: ClientControlRouteTarget,
  navigation: ClientControlNavigation,
): void => {
  navigation.navigate(navigation.href(target))
}
