// A person's GitHub identity, for real avatars on their messages.

/** Fractal person refs whose GitHub login is known. A person missing here keeps a monogram; no login is guessed from a ref. */
const githubLogins: Readonly<Record<string, string>> = {
  'person/schickling': 'schickling',
}

/** The person's GitHub avatar, from the one origin the web CSP allows for images, sized for a 20px head at 2x. */
export const personAvatarUrl = (ref: string | undefined): string | undefined => {
  const login = ref === undefined ? undefined : githubLogins[ref]
  return login === undefined ? undefined : `https://avatars.githubusercontent.com/${encodeURIComponent(login)}?s=40`
}
