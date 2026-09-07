# Publicar Misku Native Views

## Validar sin publicar

La ejecución manual del workflow **Release** es un ensayo: ejecuta los mismos checks, genera y prueba los archivos, y guarda el artifact `distribution`. No publica en npm ni crea una release de GitHub. También se generan los mismos archivos en cada CI del PR.

## Publicar una versión

1. Actualiza `package.json`, `src-tauri/Cargo.toml`, `src-tauri/tauri.conf.json` y `Cargo.lock`. La validación rechaza cualquier diferencia.
2. Integra el PR con **CI required** en verde.
3. Crea un tag `v<version>` sobre ese commit de `main` y súbelo. Por ejemplo, para publicar la versión 0.3.0 ya revisada:

```powershell
git switch main
git pull --ff-only
git tag v0.3.0
git push origin v0.3.0
```

El workflow vuelve a validar, compila una sola vez para ambos formatos, prueba las instalaciones y pasa los archivos a la publicación sin recompilarlos. Comprueba que el commit pertenece a `main` y que el tag coincide exactamente. Versiones como `0.4.0-beta.1` se publican en `next` y como prerelease en GitHub; las estables van a `latest`.

La publicación usa `NPM_TOKEN` en GitHub Actions y el token efímero de GitHub para la release. El token npm debe permitir publicar `misku-native-view-cli` y satisfacer la política de 2FA del paquete. Se usa el entorno `release`. No se necesitan credenciales en PRs, builds o pruebas. npm recibe `--provenance`; GitHub genera attestations para los assets con OIDC. Estas identidades se limitan al job de publicación.

## Verificar una descarga

Compara `Get-FileHash <archivo> -Algorithm SHA256` con `SHA256SUMS`. Para verificar la procedencia de GitHub:

```powershell
gh attestation verify ./Misku-Native-Views-0.3.0-x64-setup.exe --repo JohannRojas/misku-native-view
```

`build-info.json` identifica el commit, versión, destino, hash del runtime y estado Authenticode. El instalador aún no está firmado con un certificado de Windows: no se debe presentar la attestation de GitHub como una firma Authenticode. Incorporar firma requiere configurar una identidad de firma real antes de compilar y empaquetar, y mantener los mismos bytes entre npm y el instalador.

## Recuperar una publicación parcial

Reejecuta **los jobs fallidos** de la misma ejecución; así se conservan los artifacts ya probados. Si npm ya publicó exactamente el mismo tarball, el workflow comprueba su integridad SHA-512 y continúa con GitHub. Si los bytes difieren, se detiene: publica una versión nueva. No borres ni muevas un tag publicado y no intentes sobrescribir una versión npm.

Para volver a una versión anterior, reinstala su instalador o `pnpm install -g misku-native-view-cli@<version>`. Los runtimes administrados son versionados; no se eliminan automáticamente cookies, perfiles ni sesiones. Haz copia de `apps.toml` antes de cualquier migración manual. No se ha añadido actualización automática de las apps instaladas.
