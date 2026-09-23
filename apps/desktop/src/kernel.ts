/**
 * Kernel client for the trusted main frame. All traffic goes through the
 * single Tauri command `kernel_rpc` guarded by the bootstrap token
 * (ADR-0003). Plugin iframes never see this module.
 */
import { invoke } from '@tauri-apps/api/core';
import type { RpcRequest, RpcResponse } from '@workbench-zero/protocol';
import { KernelRpcError, Methods } from '@workbench-zero/protocol';

let token: string | null = null;
let nextId = 1;

export function issueToken(): Promise<string> {
  const req: RpcRequest = { id: nextId++, method: Methods.app.issueToken, params: {} };
  return invoke<RpcResponse>('kernel_rpc', { token: '', payload: req }).then((res) => {
    if (!res.ok) throw new KernelRpcError(res.error.code, res.error.message);
    token = res.result as string;
    return token;
  });
}

export async function rpc<T = unknown>(
  method: string,
  params: Record<string, unknown> = {},
): Promise<T> {
  if (token === null) throw new Error('kernel token not issued yet');
  const req: RpcRequest = { id: nextId++, method, params };
  const res = await invoke<RpcResponse>('kernel_rpc', { token, payload: req });
  if (res.ok) return res.result as T;
  throw new KernelRpcError(res.error.code, res.error.message);
}

/** Set the global Quick Capture shortcut on/off (Alt+Space). */
export function setGlobalCapture(enabled: boolean): Promise<void> {
  return invoke('set_global_capture', { enabled });
}
