-- Custom Setup: builds the Rust core with cargo and links its static archive.
--
-- The package ships the Rust sources (rust/) so that `cabal install stem-mqtt` is
-- self-contained: no pre-built native library to download or locate. It needs a
-- Rust toolchain (cargo) on PATH — the same kind of build-time requirement as a C
-- compiler or pkg-config for other bindings packages.
module Main (main) where

import Control.Monad (unless, when)
import Distribution.PackageDescription
import Distribution.Simple
import Distribution.Simple.LocalBuildInfo
import Distribution.Simple.Setup (ConfigFlags)
import System.Directory (doesFileExist, getCurrentDirectory)
import System.Exit (ExitCode (..), exitFailure)
import System.FilePath ((</>))
import System.IO (hPutStrLn, stderr)
import System.Process (CreateProcess (..), proc, readCreateProcessWithExitCode)

main :: IO ()
main = defaultMainWithHooks simpleUserHooks {confHook = rustConfHook}

rustConfHook ::
  (GenericPackageDescription, HookedBuildInfo) ->
  ConfigFlags ->
  IO LocalBuildInfo
rustConfHook pkg flags = do
  lbi <- confHook simpleUserHooks pkg flags
  root <- getCurrentDirectory
  let rustDir = root </> "rust"
      archive = rustDir </> "target" </> "release" </> "libmqtt_broker.a"
  built <- doesFileExist archive
  unless built $ do
    putStrLn "stem-mqtt: building the Rust core with cargo (the first build takes a minute or two)..."
    (code, out, err) <-
      readCreateProcessWithExitCode
        (proc "cargo" ["build", "--release", "--locked", "-p", "stem-mqtt-broker"]) {cwd = Just rustDir}
        ""
    when (code /= ExitSuccess) $ do
      hPutStrLn stderr $
        "stem-mqtt: `cargo build` failed — is a Rust toolchain installed (https://rustup.rs)?\n"
          ++ out
          ++ err
      exitFailure
  -- Link the static archive by absolute path (the cdylib next to it would win a plain -l).
  let withArchive lib = lib {libBuildInfo = (libBuildInfo lib) {ldOptions = ldOptions (libBuildInfo lib) ++ [archive]}}
      descr = localPkgDescr lbi
  return lbi {localPkgDescr = descr {library = fmap withArchive (library descr)}}
