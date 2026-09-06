export function operationErrorSummary(message: string): string {
  if (message.includes('DEDICATED_SERVER') && message.includes('net/minecraft/client/')) {
    return 'A client-only mod was included in the server files. This release needs the publisher’s dedicated server pack.';
  }
  const summary = (message.split(/(?:Relevant|Latest) startup output:/, 1)[0] ?? message).trim();
  return summary.length > 280 ? `${summary.slice(0, 277)}…` : summary;
}

export function OperationError({ message }: { message: string }) {
  const summary = operationErrorSummary(message);
  return (
    <div class="operation-error">
      <p>{summary}</p>
      {summary !== message && (
        <details>
          <summary>Technical details</summary>
          <pre tabIndex={0} aria-label="Operation error details">{message}</pre>
        </details>
      )}
    </div>
  );
}
