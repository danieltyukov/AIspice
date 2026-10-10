import { memo } from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkGfm from "remark-gfm";

/*
 * Model output rendered as markdown with GitHub tables and lists. Raw HTML is
 * dropped (skipHtml, and no rehype-raw), unsafe URLs are removed by
 * react-markdown's default transform, and images are not fetched: a model
 * cannot make the app load a remote resource.
 */

const components: Components = {
  a: ({ href, children }) => (
    <a href={href} target="_blank" rel="noopener noreferrer">
      {children}
    </a>
  ),
  img: ({ alt }) => <span className="md-img">[image{alt ? `: ${alt}` : ""}]</span>,
  table: ({ children }) => (
    <div className="md-table">
      <table>{children}</table>
    </div>
  ),
};

export const Markdown = memo(function Markdown({ text }: { text: string }) {
  return (
    <div className="md">
      <ReactMarkdown remarkPlugins={[remarkGfm]} skipHtml components={components}>
        {text}
      </ReactMarkdown>
    </div>
  );
});
