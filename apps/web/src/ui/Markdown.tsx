import React from "react";

function isSafeUrl(url: string): boolean {
  const trimmed = url.trim().toLowerCase();
  return (
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://") ||
    trimmed.startsWith("mailto:")
  );
}

function parseInline(text: string): React.ReactNode[] {
  const nodes: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  const inlineRegex =
    /(`([^`]+)`)|(\*\*([^*]+)\*\*|__([^_]+)__)|(\*([^*]+)\*|_([^_]+)_)|(\[([^\]]+)\]\(([^)]+)\))|(\n)/;

  while (remaining.length > 0) {
    const match = remaining.match(inlineRegex);
    if (!match || match.index === undefined) {
      nodes.push(remaining);
      break;
    }

    if (match.index > 0) {
      nodes.push(remaining.slice(0, match.index));
    }

    const [
      full,
      codeFull,
      codeContent,
      boldFull,
      bold1,
      bold2,
      italicFull,
      italic1,
      italic2,
      linkFull,
      linkText,
      linkUrl,
      newline,
    ] = match;

    if (codeFull) {
      nodes.push(
        <code
          key={key++}
          className="px-1 py-0.5 rounded bg-slate-100 dark:bg-slate-800 text-xs font-mono text-slate-800 dark:text-slate-200"
        >
          {codeContent}
        </code>
      );
    } else if (boldFull) {
      const boldText = bold1 || bold2;
      nodes.push(
        <strong key={key++} className="font-bold">
          {parseInline(boldText)}
        </strong>
      );
    } else if (italicFull) {
      const itText = italic1 || italic2;
      nodes.push(
        <em key={key++} className="italic">
          {parseInline(itText)}
        </em>
      );
    } else if (linkFull) {
      if (isSafeUrl(linkUrl)) {
        nodes.push(
          <a
            key={key++}
            href={linkUrl}
            target="_blank"
            rel="noopener noreferrer"
            aria-label={`${linkText} (${linkUrl})`}
            className="text-blue-600 dark:text-blue-400 underline hover:text-blue-800 dark:hover:text-blue-300"
          >
            {linkText}
          </a>
        );
      } else {
        nodes.push(linkFull);
      }
    } else if (newline) {
      nodes.push(<br key={key++} />);
    }

    remaining = remaining.slice(match.index + full.length);
  }

  return nodes;
}

export interface MarkdownProps {
  content: string;
  className?: string;
}

export const Markdown: React.FC<MarkdownProps> = ({ content, className = "" }) => {
  if (!content) return null;

  const rawBlocks = content.split(/\r?\n\s*\r?\n/);
  const elements: React.ReactNode[] = [];

  rawBlocks.forEach((block, blockIdx) => {
    const trimmed = block.trim();
    if (!trimmed) return;

    const lines = trimmed.split(/\r?\n/);
    const isBulletList = lines.every((l) => /^\s*[-*]\s+/.test(l));
    const isNumberedList = lines.every((l) => /^\s*\d+\.\s+/.test(l));

    if (isBulletList) {
      elements.push(
        <ul key={blockIdx} className="list-disc list-inside space-y-0.5 my-1.5">
          {lines.map((line, lineIdx) => {
            const itemText = line.replace(/^\s*[-*]\s+/, "");
            return <li key={lineIdx}>{parseInline(itemText)}</li>;
          })}
        </ul>
      );
    } else if (isNumberedList) {
      elements.push(
        <ol key={blockIdx} className="list-decimal list-inside space-y-0.5 my-1.5">
          {lines.map((line, lineIdx) => {
            const itemText = line.replace(/^\s*\d+\.\s+/, "");
            return <li key={lineIdx}>{parseInline(itemText)}</li>;
          })}
        </ol>
      );
    } else {
      elements.push(
        <p key={blockIdx} className="my-1.5 leading-relaxed">
          {parseInline(block)}
        </p>
      );
    }
  });

  return (
    <div className={`markdown-content text-slate-800 dark:text-slate-200 text-xs ${className}`}>
      {elements}
    </div>
  );
};
