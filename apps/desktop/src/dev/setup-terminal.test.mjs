import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { WebLinksAddon as RealWebLinksAddon } from "@xterm/addon-web-links";

const source = readFileSync(new URL("../components/Settings.tsx", import.meta.url), "utf8");
const parsed = ts.createSourceFile("Settings.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
const component = parsed.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "SetupTerminal");
const code = ts.transpileModule(component.getText(parsed), {
  compilerOptions: { target: ts.ScriptTarget.ES2022, jsx: ts.JsxEmit.React },
}).outputText;

function mount(open = async () => {}) {
  const terminals = [];
  const opened = [];
  const errors = [];
  let cleanup;
  class Terminal {
    addons = [];
    cols = 80;
    rows = 14;
    constructor(options) { this.options = options; terminals.push(this); }
    loadAddon(addon) { this.addons.push(addon); addon.activate?.(this); }
    registerLinkProvider(provider) { this.linkProvider = provider; return { dispose() {} }; }
    open() {}
    onData() {}
    write() {}
    focus() {}
    dispose() { this.disposed = true; }
  }
  class FitAddon { fit() {} }
  class WebLinksAddon extends RealWebLinksAddon {
    constructor(handler) { super(handler); this.handler = handler; }
  }
  const bindings = {
    Terminal, FitAddon, WebLinksAddon,
    openUrl: (url) => { opened.push(url); return open(url); },
    toast: (message) => errors.push(message),
    useRef: () => ({ current: {} }),
    useState: () => [false, () => {}],
    useEffect: (effect) => { cleanup = effect(); },
    xtermTheme: () => ({}),
    followTheme: () => () => {},
    crypto: { randomUUID: () => "test" },
    api: { setupTerminalOpen: async () => {}, terminalKill: async () => {}, terminalWrite: async () => {} },
    React: { createElement: () => null },
    Loader2: () => null,
  };
  const render = new Function(...Object.keys(bindings), `${code}; return SetupTerminal;`)(...Object.values(bindings));
  render({ title: "Signing in", command: "claude auth login", onClose() {} });
  return { term: terminals[0], opened, errors, cleanup, WebLinksAddon };
}

const url = `https://claude.com/cai/oauth/authorize?code=true&redirect_uri=https%3A%2F%2Fplatform.claude.com%2Foauth%2Fcode%2Fcallback&state=${"x".repeat(160)}&code_challenge=test`;

test("setup terminal registers clickable web links without automatically opening output", () => {
  const mounted = mount();
  assert.ok(mounted.term.addons.some((addon) => addon instanceof mounted.WebLinksAddon), "setup terminal is missing its web link provider");
  assert.deepEqual(mounted.opened, []);
  mounted.cleanup();
  assert.equal(mounted.term.disposed, true);
});

test("long OAuth web links and OSC 8 hyperlinks use the native browser opener", () => {
  const mounted = mount();
  const addon = mounted.term.addons.find((addon) => addon instanceof mounted.WebLinksAddon);
  assert.ok(addon, "setup terminal is missing its web link provider");
  addon.handler({}, url);
  assert.equal(typeof mounted.term.options.linkHandler?.activate, "function", "OSC 8 hyperlinks need the native opener too");
  mounted.term.options.linkHandler.activate({}, url);
  assert.deepEqual(mounted.opened, [url, url]);
});

test("terminal links cannot launch non-web schemes", () => {
  const mounted = mount();
  assert.equal(typeof mounted.term.options.linkHandler?.activate, "function");
  for (const uri of ["javascript:alert(1)", "file:///etc/passwd", "suneiro://command"]) {
    mounted.term.options.linkHandler.activate({}, uri);
  }
  assert.deepEqual(mounted.opened, []);
});

test("native opener failures are visible without exposing OAuth parameters", async () => {
  const mounted = mount(async () => { throw new Error(`cannot open ${url}`); });
  const addon = mounted.term.addons.find((addon) => addon instanceof mounted.WebLinksAddon);
  assert.ok(addon, "setup terminal is missing its web link provider");
  addon.handler({}, url);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(mounted.errors.length, 1);
  assert.match(mounted.errors[0], /open.*link/i);
  assert.ok(!mounted.errors[0].includes(url));
});

test("clicking any wrapped OAuth URL row opens the complete URL", () => {
  const mounted = mount();
  const text = `If the browser didn't open, visit: ${url}`;
  const cols = 80;
  const lines = Array.from({ length: Math.ceil(text.length / cols) }, (_, index) => {
    const content = text.slice(index * cols, (index + 1) * cols);
    return {
      isWrapped: index > 0,
      length: cols,
      translateToString: () => content,
      getCell: (column, cell) => { cell.chars = content[column] ?? ""; return cell; },
    };
  });
  mounted.term.cols = cols;
  mounted.term.buffer = { active: {
    getLine: (index) => lines[index],
    getNullCell: () => ({ chars: "", getChars() { return this.chars; }, getWidth: () => 1 }),
  } };
  assert.ok(mounted.term.linkProvider, "setup terminal is missing its web link provider");
  for (let row = 1; row <= lines.length; row++) {
    let links;
    mounted.term.linkProvider.provideLinks(row, (found) => { links = found; });
    assert.equal(links.length, 1);
    assert.equal(links[0].text, url);
    links[0].activate({}, links[0].text);
  }
  assert.deepEqual(mounted.opened, lines.map(() => url));
});
