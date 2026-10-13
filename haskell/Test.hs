{- | The Haskell port's properties, run on every core (+RTS -N) so the races are real:
     sms-hs-test SPEC_JSON +RTS -N
fold_equivalence and monitor_monotone (the spec's names), after a check that the constants Sim.hs
transcribes from spec.json agree with it, that the flags parse like `sms run`'s and that the parallel fold
equals the sequential one. -}
{-# LANGUAGE OverloadedStrings #-}
module Main (main) where

import Control.Monad (forM, unless)
import Data.Aeson (Value, eitherDecodeFileStrict, withObject, (.:))
import Data.Aeson.Types (Parser, parseEither)
import Data.Either (isLeft)
import Data.IORef (modifyIORef', newIORef, readIORef)
import qualified Data.Map.Strict as Map
import Fate (Params (..), Summary (..), maxLen, runRange)
import Sim (Config (..), alphabet, params, parRange, parse, run)
import System.Environment (getArgs)
import System.Exit (exitFailure)
import System.Info (os)
import Test.QuickCheck (Args (..), Gen, Property, choose, counterexample, elements, forAll, ioProperty, oneof)
import Test.QuickCheck (Result (output), isSuccess, quickCheckWithResult, stdArgs, (===))
import Test.QuickCheck.Random (mkQCGen)

-- | Two regimes: short runs that sleep (the RTS rounds each sleep up to 1 ms, so a case stays short), and
-- wide runs with no waits, where the producer, the Chan and the slots race on every core. Attempts,
-- p and the seed span the spec; T is 1 ms, below the CLI's bound, so even short runs see many ticks.
configs :: Gen Config
configs = oneof [shape 200 [0, 0, 400] [0, 1], shape 2000 [0] [0]]
  where
    shape top mus betas = do
      p <- Params <$> choose (0, 2 ^ (53 :: Int) - 1) <*> elements mus
        <*> oneof [elements [0, 1000000], choose (0, 1000000)] <*> choose (1, 10) <*> elements betas
      Config <$> choose (0, top) <*> choose (1, 64) <*> choose (1, 64) <*> pure 1 <*> pure p

fold :: Config -> Summary
fold cfg = runRange (fate cfg) 0 (n cfg)

-- | fold_equivalence: on every core, the concurrent run ends and its Summary equals the pure fold.
prop_fold_equivalence :: Property
prop_fold_equivalence = forAll configs $ \cfg -> ioProperty $ (=== fold cfg) <$> run cfg (\_ _ -> pure ())

-- | monitor_monotone: the first snapshot is empty at t = 0, none decreases in time or in any count, and the
-- last is the total, which is the fold.
prop_monitor_monotone :: Property
prop_monitor_monotone = forAll configs $ \cfg -> ioProperty $ do
  seen <- newIORef []
  total <- run cfg (\t s -> modifyIORef' seen ((t, s) :))
  latest <- readIORef seen
  let snaps = reverse latest
  pure $ counterexample (show (cfg, length snaps, take 3 snaps, take 1 latest)) $
    take 1 snaps == [(0, mempty)] && and (zipWith le snaps (drop 1 snaps))
      && map snd (take 1 latest) == [total] && total == fold cfg
  where
    le (t, Summary s f a u) (t', Summary s' f' a' u') = t <= t' && s <= s' && f <= f' && a <= a' && u <= u'

-- | Sim.hs's alphabet, max_len and (default, min, max) of every parameter equal spec.json's.
constants :: Value -> Parser Bool
constants = withObject "spec" $ \o -> do
  ps <- o .: "params" :: Parser (Map.Map String Value)
  want <- traverse (withObject "param" $ \q -> (,,) <$> q .: "default" <*> q .: "min" <*> q .: "max") ps
  (abc, ml) <- (,) <$> o .: "alphabet" <*> o .: "max_len"
  pure $ abc == alphabet && ml == maxLen && want == Map.fromList [(k, (d, lo, hi)) | (k, d, lo, hi) <- params]

-- | Both flag spellings parse and bad input is refused.
flags :: Bool
flags = case parse [] of
  Left _ -> False
  Right d ->
    let want = Right d {n = 7, fate = (fate d) {meanUs = 0}}
     in all ((== want) . parse . words) ["--n=7 --mean-us 0", "--n 7 --mean-us=0"]
          && all (isLeft . parse . words)
            ["--n -1", "--n", "--chunk 5", "--senders 0", "--n 0x10", "--interval-ms 99"]

-- | The parallel fold equals the sequential fold for several N and chunk counts, and at the defaults it gives
-- the pinned answer (adi-sms docs/DESIGN.md; exact with Linux's libm).
parallel :: Bool
parallel = either (const False) (check . fate) (parse [])
  where
    check d = (os /= "linux" || parRange d 8 0 1000 == Summary 893 107 1000 99301920)
      && and [parRange p k 0 m == runRange p 0 m | m <- [0, 1, 7, 1000, 20000], k <- [1, 2, 3, 8, 64]]
      where p = d {failPpm = 300000, maxAttempts = 3}

main :: IO ()
main = do
  args <- getArgs
  specPath <- case args of [path] -> pure path; _ -> fail "usage: sms-hs-test SPEC_JSON [+RTS -N]"
  agree <- eitherDecodeFileStrict specPath >>= either fail pure . (>>= parseEither constants)
  putStrLn ("constants and flags: " <> if agree && flags then "agree" else "DIFFER")
  putStrLn ("parallel fold: " <> if parallel then "agrees" else "DIFFERS")
  let qc = stdArgs {replay = Just (mkQCGen 20261009, 0), maxSuccess = 100, chatty = False}
  rs <- forM [("fold_equivalence", prop_fold_equivalence), ("monitor_monotone", prop_monitor_monotone)] $
    \(name, p) -> quickCheckWithResult qc p >>= \r -> r <$ putStr (name <> ": " <> output r)
  unless (agree && flags && parallel && all isSuccess rs) exitFailure
