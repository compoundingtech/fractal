import { useCallback, useEffect, useLayoutEffect, useRef, useState } from 'react';

/** Keep typing local. Profile changes replace the draft; leaving retains unfinished text. */
export function useGatewayDraft(saved: string, retain: (value: string) => void) {
  const [value, setValue] = useState(saved);
  const current = useRef(saved), dirty = useRef(false);
  useLayoutEffect(() => {
    current.current = saved; dirty.current = false; setValue(saved);
  }, [saved]);
  useEffect(() => () => { if (dirty.current) retain(current.current); }, [retain]);
  const change = useCallback((next: string) => {
    current.current = next; dirty.current = true; setValue(next);
  }, []);
  // A submit reads the latest native change, including before React commits its next render.
  const read = useCallback(() => current.current, []);
  return { value, change, read };
}
