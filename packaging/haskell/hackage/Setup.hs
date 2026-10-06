{-# LANGUAGE CPP #-}

-- Custom Setup: builds the Rust core with cargo and links its static archive.
--
-- The package ships the Rust sources (rust/) so that `cabal install stem-mqtt` is
-- self-contained: no pre-built native library to download or locate. It needs a
-- Rust toolchain (cargo) on PATH — the same kind of build-time requirement as a C
-- compiler or pkg-config for other bindings packages.
--
-- The archive is linked with `-lmqtt_broker`, from a directory that holds *only* the static
-- `.a` (the cdylib next to it in target/release would win a plain `-l`). Using `-l` rather than
-- naming the file keeps GNU ld happy: libraries must come *after* the objects that need them.
-- At install time the archive is copied next to the Haskell library so that packages built
-- later against the installed `stem-mqtt` still find it.
module Main (main) where

import Control.Monad (unless, when)
import Distribution.PackageDescription
import Distribution.Simple
import Distribution.Simple.InstallDirs (CopyDest (..), libdir)
import Distribution.Simple.LocalBuildInfo
import Distribution.Simple.Setup (ConfigFlags, CopyFlags, copyDest, fromFlag)
#if MIN_VERSION_Cabal(3,14,0)
import Distribution.Utils.Path (makeSymbolicPath)
#endif
import System.Directory (copyFile, createDirectoryIfMissing, doesFileExist, getCurrentDirectory)
import System.Exit (ExitCode (..), exitFailure)
import System.FilePath ((</>))
import System.IO (hPutStrLn, stderr)
import System.Process (CreateProcess (..), proc, readCreateProcessWithExitCode)

-- Cabal 3.14 turned library directories into 'SymbolicPath's.
#if MIN_VERSION_Cabal(3,14,0)
libDir path = makeSymbolicPath path
#else
libDir path = path
#endif

archiveName :: FilePath
archiveName = "libmqtt_broker.a"

-- | Directory (relative to the package root) holding only the static archive.
staticDir :: FilePath -> FilePath
staticDir root = root </> "rust" </> "target" </> "static"

main :: IO ()
main =
  defaultMainWithHooks
    simpleUserHooks
      { confHook = rustConfHook
      , postCopy = installArchive
      }

rustConfHook ::
  (GenericPackageDescription, HookedBuildInfo) ->
  ConfigFlags ->
  IO LocalBuildInfo
rustConfHook pkg flags = do
  lbi <- confHook simpleUserHooks pkg flags
  root <- getCurrentDirectory
  let rustDir = root </> "rust"
      built = rustDir </> "target" </> "release" </> archiveName
      staticLib = staticDir root
  haveArchive <- doesFileExist built
  unless haveArchive $ do
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
  createDirectoryIfMissing True staticLib
  copyFile built (staticLib </> archiveName)
  let withArchive lib =
        lib
          { libBuildInfo =
              (libBuildInfo lib)
                { extraLibDirs = extraLibDirs (libBuildInfo lib) ++ [libDir staticLib]
                , extraLibs = extraLibs (libBuildInfo lib) ++ ["mqtt_broker"]
                }
          }
      descr = localPkgDescr lbi
  return lbi {localPkgDescr = descr {library = fmap withArchive (library descr)}}

-- | Put the archive next to the installed Haskell library.
installArchive :: Args -> CopyFlags -> PackageDescription -> LocalBuildInfo -> IO ()
installArchive _ flags pkg lbi = do
  root <- getCurrentDirectory
  let dest = libdir (absoluteInstallDirs pkg lbi (fromFlag (copyDest flags)))
  createDirectoryIfMissing True dest
  copyFile (staticDir root </> archiveName) (dest </> archiveName)
