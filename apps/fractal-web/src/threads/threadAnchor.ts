/** Every rendered message has one anchor, independent of its author and the page's freshness. */
export const threadAnchorAttribute = 'data-thread-anchor'
export const threadAnchorSelector = `[${threadAnchorAttribute}]`

/** Natural read receipts still qualify only live incoming replies, separately from navigable anchors. */
export const threadReadEligibleAttribute = 'data-thread-read-eligible'
