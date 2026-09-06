import render from 'preact-render-to-string';
import { describe, expect, it } from 'vitest';
import { OperationError, operationErrorSummary } from './operation-error';

describe('operation errors', () => {
  it('explains client-only crashes and preserves the log in collapsed details', () => {
    const message = 'Minecraft exited. Relevant startup output: Drippy attempted net/minecraft/client/gui/screens/Screen for invalid dist DEDICATED_SERVER';
    const html = render(<OperationError message={message} />);
    expect(html).toContain('A client-only mod');
    expect(html).toContain('<details>');
    expect(html).not.toContain('<details open');
    expect(html).toContain(message);
  });
  it('keeps short errors simple and bounds long summaries without losing details', () => {
    expect(render(<OperationError message="Port is busy." />)).not.toContain('<details>');
    expect(operationErrorSummary('x'.repeat(5000))).toHaveLength(278);
    const html = render(<OperationError message={'<script>' + 'x'.repeat(5000)} />);
    expect(html).not.toContain('<script>');
    expect(html).toContain('&lt;script>');
  });
});
