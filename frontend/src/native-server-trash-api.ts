import { expectRecord, expectString, requestJson } from './api';

export function purgeTrashedNativeServer(
  trashId: string,
  confirmationName: string,
  csrfToken: string,
): Promise<{ instanceId: string }> {
  return requestJson(
    `/api/v1/servers/removed/${encodeURIComponent(trashId)}`,
    (value) => {
      const root = expectRecord(value, 'purged server result');
      if (root.purged !== true) throw new Error('Permanent deletion was not verified');
      return { instanceId: expectString(root, 'instance_id', 'purged server result') };
    },
    {
      method: 'DELETE',
      body: { confirmation_name: confirmationName },
      csrfToken,
      timeoutMs: 310_000,
    },
  );
}
