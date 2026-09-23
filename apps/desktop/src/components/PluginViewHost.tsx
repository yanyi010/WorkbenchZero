import { useEffect, useRef } from 'react';
import { pluginHost } from '../pluginHost';

/**
 * Hosts one plugin view iframe. Mount attaches the frame; unmount detaches.
 * The frame itself is owned by pluginHost so it survives tab switching
 * without reloading plugin state.
 */
export function PluginViewHost({ pluginId, viewId }: { pluginId: string; viewId: string }) {
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!ref.current) return;
    pluginHost.attachViewFrame(ref.current, pluginId, viewId);
    return () => {
      pluginHost.detachViewFrame(pluginId, viewId);
    };
  }, [pluginId, viewId]);

  return <div ref={ref} className="ed-view-frame" role="region" aria-label={`${pluginId} view`} />;
}
