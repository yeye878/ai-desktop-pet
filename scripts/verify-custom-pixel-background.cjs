const fs = require("fs");
const vm = require("vm");
const ts = require("typescript");

const source = fs.readFileSync("src/services/customPixelPetBackground.ts", "utf8");
const compiled = ts.transpileModule(source, {
  compilerOptions: {
    module: ts.ModuleKind.CommonJS,
    target: ts.ScriptTarget.ES2021,
  },
}).outputText;
const moduleContext = { exports: {} };

vm.runInNewContext(compiled, {
  module: moduleContext,
  exports: moduleContext.exports,
  Uint8Array,
  Uint8ClampedArray,
  Map,
  Set,
  Math,
});

const { removeEdgeBackground } = moduleContext.exports;

function createImageData(width, height, fill) {
  const data = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < data.length; i += 4) {
    data[i] = fill[0];
    data[i + 1] = fill[1];
    data[i + 2] = fill[2];
    data[i + 3] = fill[3];
  }
  return { data, width, height };
}

function fillRect(imageData, x0, y0, width, height, color) {
  for (let y = y0; y < y0 + height; y++) {
    for (let x = x0; x < x0 + width; x++) {
      const index = (y * imageData.width + x) * 4;
      imageData.data[index] = color[0];
      imageData.data[index + 1] = color[1];
      imageData.data[index + 2] = color[2];
      imageData.data[index + 3] = color[3];
    }
  }
}

function alphaAt(imageData, x, y) {
  return imageData.data[(y * imageData.width + x) * 4 + 3];
}

function assert(name, condition) {
  if (!condition) throw new Error(name);
}

const centered = createImageData(40, 40, [40, 190, 90, 255]);
fillRect(centered, 12, 12, 16, 16, [220, 40, 55, 255]);
removeEdgeBackground(centered);
assert("centered background corner is transparent", alphaAt(centered, 0, 0) === 0);
assert("centered pure-color subject remains opaque", alphaAt(centered, 20, 20) === 255);

const sideTouching = createImageData(40, 40, [40, 190, 90, 255]);
fillRect(sideTouching, 0, 12, 20, 16, [220, 40, 55, 255]);
removeEdgeBackground(sideTouching);
assert("side-touching background corner is transparent", alphaAt(sideTouching, 39, 39) === 0);
assert("side-touching pure-color subject remains opaque", alphaAt(sideTouching, 6, 20) === 255);

const splitCorners = createImageData(40, 40, [40, 190, 90, 255]);
fillRect(splitCorners, 0, 0, 40, 12, [220, 40, 55, 255]);
removeEdgeBackground(splitCorners);
assert("ambiguous two-corner subject remains opaque", alphaAt(splitCorners, 20, 6) === 255);
assert("ambiguous background is preserved instead of destructive clearing", alphaAt(splitCorners, 20, 30) === 255);

console.log("custom pixel background checks passed");
