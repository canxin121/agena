import MagicString from 'magic-string'
import ts from 'typescript'
import type { Plugin } from 'vite'

/** Remixicon's published ESM does not mark its component constructors as pure.
 * Annotate only those declarations so unused icons can be discarded.
 */
export function remixiconTreeShaking(): Plugin {
  return {
    name: 'agena-remixicon-component-purity',
    enforce: 'pre',
    transform(code, id) {
      const path = id.split('?')[0]!.replaceAll('\\', '/')
      if (!path.endsWith('/node_modules/@remixicon/vue/index.mjs')) return
      const source = ts.createSourceFile(path, code, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS)
      let factory: string | undefined
      for (const statement of source.statements) {
        if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier)) continue
        if (statement.moduleSpecifier.text !== 'vue') continue
        const bindings = statement.importClause?.namedBindings
        if (!bindings || !ts.isNamedImports(bindings)) continue
        for (const binding of bindings.elements) {
          if ((binding.propertyName ?? binding.name).text === 'defineComponent') factory = binding.name.text
        }
      }
      if (!factory) return
      const output = new MagicString(code)
      let annotated = false
      for (const statement of source.statements) {
        if (!ts.isVariableStatement(statement)) continue
        for (const declaration of statement.declarationList.declarations) {
          const call = declaration.initializer
          if (!call || !ts.isCallExpression(call) || !ts.isIdentifier(call.expression)) continue
          if (call.expression.text !== factory || call.arguments.length !== 1) continue
          const options = call.arguments[0]!
          if (!ts.isObjectLiteralExpression(options)) continue
          const name = options.properties.find(
            (property) => ts.isPropertyAssignment(property) && property.name.getText(source) === 'name',
          )
          if (!name || !ts.isPropertyAssignment(name) || !ts.isStringLiteral(name.initializer)) continue
          if (!/^Ri[A-Z0-9]/.test(name.initializer.text)) continue
          output.appendLeft(call.getStart(source), '/*#__PURE__*/ ')
          annotated = true
        }
      }
      if (!annotated) return
      return {
        code: output.toString(),
        map: output.generateMap({ source: path, includeContent: true, hires: 'boundary' }),
      }
    },
  }
}
