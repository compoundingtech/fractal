// Owned native presentations share a visibility barrier. Each owner releases only
// its own lease, so closing a nested sheet cannot uncover the underlying content.
export const createPresentationBarrier = (): {
  isOccluded(): boolean;
  acquire(): () => void;
  watch(changed: () => void): () => void;
} => {
  let owners = 0;
  const listeners = new Set<() => void>();
  const notify = () => { for (const changed of listeners) changed(); };
  return {
    isOccluded: () => owners > 0,
    acquire: () => {
      owners++;
      if (owners === 1) notify();
      let released = false;
      return () => {
        if (released) return;
        released = true;
        owners--;
        if (owners === 0) notify();
      };
    },
    watch: changed => { listeners.add(changed); return () => { listeners.delete(changed); }; },
  };
};

export const ownedPresentations = createPresentationBarrier();
