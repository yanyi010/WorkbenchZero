/**
 * EigenDesk shell entry. Bootstrap order (ADR-0003):
 *  1. issue the kernel token (first caller wins — before any iframe),
 *  2. install the kernel push inbox + Quick Capture global hook,
 *  3. boot the store, then render.
 */
import React from 'react';
import ReactDOM from 'react-dom/client';
import '@eigendesk/ui-kit/tokens.css';
import './styles.css';
import type { PushMessage } from '@eigendesk/protocol';
import { issueToken } from './kernel';
import { pluginHost } from './pluginHost';
import { useApp } from './store';
import { App } from './App';

async function start(): Promise<void> {
  pluginHost.start();

  window.__kernelInbox = (batch: PushMessage[]) => {
    useApp.getState().handleKernelPush(batch);
  };
  window.__quickCapture = () => {
    useApp.getState().setOverlay('capture');
  };

  try {
    await issueToken();
  } catch (err) {
    // The app cannot work without the kernel, but showing the error is
    // better than a blank window.
    useApp.setState({ booted: true, bootError: String(err) });
  }

  await useApp.getState().boot();
  // Cache manifests for frame entry resolution.
  for (const p of useApp.getState().plugins) {
    pluginHost.cacheManifest(p.manifest);
  }
}

declare global {
  interface Window {
    __kernelInbox?: (batch: PushMessage[]) => void;
    __quickCapture?: () => void;
  }
}

ReactDOM.createRoot(document.getElementById('root')!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

void start();
