import { readFile, writeFile } from 'fs/promises'

const version = process.argv[2].replace(/^v/, '')
const packageJson = JSON.parse(await readFile('package.json', 'utf8'))
packageJson.version = version
// Update platform-specific deps to match release version
for (const dep of Object.keys(packageJson.optionalDependencies)) {
  if (dep.startsWith(packageJson.name)) {
    packageJson.optionalDependencies[dep] = `^${version}`
  }
}
await writeFile('package.json', JSON.stringify(packageJson, null, 2))
