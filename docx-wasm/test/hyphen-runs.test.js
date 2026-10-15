const w = require("../dist/node");
const Zip = require("adm-zip");

describe("hyphen run elements", () => {
  test.each([
    ["addNoBreakHyphen", "noBreakHyphen"],
    ["addSoftHyphen", "softHyphen"],
  ])("%s survives the JavaScript writer and WASM reader", (method, element) => {
    const run = new w.Run().addText("well");
    run[method]().addText("known");
    const buffer = Buffer.from(
      new w.Docx().addParagraph(new w.Paragraph().addRun(run)).build()
    );
    const xml = new Zip(buffer).readAsText("word/document.xml");
    expect(xml).toMatch(new RegExp(`<w:${element}\\s*/>`));
    const children = w.readDocx(buffer).document.children[0].data.children[0].data.children;
    expect(children.map((child) => child.type)).toEqual(["text", element, "text"]);
    expect(children[0].data.text).toBe("well");
    expect(children[2].data.text).toBe("known");
    expect(children[1]).toEqual({ type: element });
  });
});
