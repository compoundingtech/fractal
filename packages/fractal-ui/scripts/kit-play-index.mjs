const isObject = value => value !== null && typeof value === 'object' && !Array.isArray(value)
const isNonemptyString = value => typeof value === 'string' && value.trim().length > 0

export function storyIdsFromIndex(index) {
  if (!isObject(index) || !isObject(index.entries)) {
    throw new Error('Storybook index must contain an entries object')
  }
  const ids = []
  for (const [key, entry] of Object.entries(index.entries)) {
    if (!isObject(entry) || !['story', 'docs'].includes(entry.type) || !isNonemptyString(entry.id) || !isNonemptyString(entry.importPath)) {
      throw new Error(`Invalid Storybook index entry ${JSON.stringify(key)}: expected type story/docs and nonempty id/importPath`)
    }
    if (entry.type === 'story') ids.push(entry.id)
  }
  if (!ids.length || new Set(ids).size !== ids.length) {
    throw new Error('Storybook index must contain unique, nonempty story IDs')
  }
  return ids.sort()
}
