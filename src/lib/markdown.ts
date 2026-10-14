/**
 * Mini-Markdown renderer, faithful to Discord's subset.
 * Everything is HTML-escaped FIRST, then formatting is applied —
 * so user input can never inject markup (XSS-safe by construction).
 *
 * Supported: **bold**, *italic*, __underline__, ~~strike~~,
 * `code`, ```codeblock```, ||spoiler||, > quote, links,
 * <@user> mentions, <#channel> mentions, <:name:id> custom emoji.
 */

interface MentionResolver {
  user(id: string): string; // display name
  channel(id: string): string; // "#name"
  isMentioningMe(id: string): boolean;
}

function esc(s: string): string {
  return s
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

export function renderMarkdown(raw: string, resolver: MentionResolver): string {
  // 1) Extract fenced code blocks first so their content is never formatted.
  const codeBlocks: string[] = [];
  let text = raw.replace(/```(?:[a-zA-Z0-9+#-]*\n)?([\s\S]*?)```/g, (_m, code: string) => {
    codeBlocks.push(code.replace(/\n$/, ''));
    return `\u0000CB${codeBlocks.length - 1}\u0000`;
  });

  // 2) Escape everything.
  text = esc(text);

  // 3) Inline code (protect from further formatting).
  const inlineCodes: string[] = [];
  text = text.replace(/`([^`\n]+)`/g, (_m, code: string) => {
    inlineCodes.push(code);
    return `\u0000IC${inlineCodes.length - 1}\u0000`;
  });

  // 4) Mentions (escaped input contains &lt;@id&gt;).
  text = text
    .replace(/&lt;@!?(\d+)&gt;/g, (_m, id: string) => {
      const name = resolver.user(id);
      const cls = resolver.isMentioningMe(id) ? 'mention self-mention' : 'mention';
      return `<span class="${cls}" data-user="${id}">@${esc(name)}</span>`;
    })
    .replace(/&lt;#(\d+)&gt;/g, (_m, id: string) => {
      const name = resolver.channel(id);
      return `<span class="mention" data-channel="${id}">${esc(name)}</span>`;
    });

  // 5) Custom emoji <:name:id> / animated <a:name:id>
  text = text.replace(/&lt;(a?):([A-Za-z0-9_~]+):(\d+)&gt;/g, (_m, _a: string, name: string, id: string) => {
    return `<img class="emoji" alt=":${name}:" title=":${name}:" src="https://cdn.discordapp.com/emojis/${id}.png" loading="lazy" />`;
  });

  // 6) Links (already escaped, so &amp; inside is fine).
  text = text.replace(/(https?:\/\/[^\s<]+)/g, (m) => `<a href="${m}" target="_blank" rel="noreferrer noopener">${m}</a>`);

  // 7) Bold / italic / underline / strike / spoiler.
  text = text
    .replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>')
    .replace(/__([^_]+)__/g, '<u>$1</u>')
    .replace(/\*([^*\n]+)\*/g, '<em>$1</em>')
    .replace(/_([^_\n]+)_/g, '<em>$1</em>')
    .replace(/~~([^~]+)~~/g, '<s>$1</s>')
    .replace(/\|\|([^|]+)\|\|/g, '<span class="spoiler">$1</span>');

  // 8) Block quotes.
  text = text
    .split('\n')
    .map((line) => (line.startsWith('&gt; ') || line === '&gt;' ? `<span class="quote-line">${line.replace(/^&gt;\s?/, '')}</span>` : line))
    .join('\n');

  // 9) Restore code.
  text = text
    .replace(/\u0000IC(\d+)\u0000/g, (_m, i: string) => `<code class="inline">${inlineCodes[Number(i)]}</code>`)
    .replace(/\u0000CB(\d+)\u0000/g, (_m, i: string) => {
      const code = codeBlocks[Number(i)];
      return `<div class="codeblock"><pre class="selectable"><code>${esc(code)}</code></pre></div>`;
    });

  return text.replace(/\n/g, '<br/>');
}
