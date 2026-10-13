{-# LANGUAGE LambdaCase #-}

-- | The Haskell port: adi-sms's src/adi_sms/pipeline.py, thread level, on GHC green threads. The fate is the
-- pinned adi-sms's spec/model/Fate.hs, imported unchanged; only the machine and the CLI live here.
-- Producer -> bounded Chan (C) -> M senders, with a monitor every T. `just conform` compares its Summary
-- with `sms fold`, byte for byte.
module Sim (Config (..), alphabet, params, parse, run, parRange, line, json, main) where

import Control.Concurrent (newChan, newQSem, readChan, signalQSem, threadDelay, waitQSem, writeChan)
import Control.Concurrent.Async (concurrently_, mapConcurrently_, race_)
import Control.DeepSeq (force)
import Control.Exception (evaluate)
import Control.Monad (forM_, forever, mfilter, replicateM, when)
import Control.Parallel.Strategies (parMap, rseq)
import Data.Char (isDigit)
import Data.IORef (modifyIORef', newIORef, readIORef)
import Data.List (stripPrefix)
import Data.Maybe (fromMaybe)
import Data.Word (Word64)
import Fate (Params (..), Summary (..), body, deliver, runRange)
import GHC.Clock (getMonotonicTime)
import System.Environment (getArgs)
import System.Exit (ExitCode (..), exitWith)
import System.IO (BufferMode (..), hPutStrLn, hSetBuffering, stderr)
import Text.Printf (printf)

-- | A run's shape: N, M, C and T here, the fate's own fields in Fate.Params.
data Config = Config {n :: Word64, senders, capacity, intervalMs :: Int, fate :: Params} deriving (Eq, Show)

-- | Transcribed from spec/Spec.dhall (rendered to src/adi_sms/spec.json); Test.hs compares them.
alphabet :: String
alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789 ."

-- | Every Config field as (name, default, min, max).
params :: [(String, Integer, Integer, Integer)]
params =
  [ ("n", 1000, 0, 1000000000), ("seed", 1, 0, 9007199254740991), ("senders", 10, 1, 10000)
  , ("capacity", 10, 1, 100000), ("mean_us", 100000, 0, 10000000), ("fail_ppm", 100000, 0, 1000000)
  , ("attempts", 1, 1, 10), ("backoff_us", 10000, 0, 1000000), ("interval_ms", 1000, 100, 60000)
  , ("rate", 0, 0, 1000000), ("workers", 1, 1, 64) -- the core's pipeline shape: parsed, then ignored
  ]

-- | The Config flags of `sms run`, `--name value` or `--name=value`, bounds-checked like fate.py's Config.
parse :: [String] -> Either String Config
parse = go [(k, d) | (k, d, _, _) <- params]
  where
    go vs [] =
      let v k = fromMaybe 0 (lookup k vs)
          int :: Num a => String -> a
          int = fromInteger . v
       in Right (Config (int "n") (int "senders") (int "capacity") (int "interval_ms")
            (Params (int "seed") (v "mean_us") (v "fail_ppm") (v "attempts") (v "backoff_us")))
    go vs (arg : rest) = do
      let (flag, inline) = stripPrefix "=" <$> break (== '=') arg
      (name, lo, hi) <- maybe (Left ("unrecognized argument: " <> arg)) Right $ do
        k <- map (\c -> if c == '-' then '_' else c) <$> stripPrefix "--" flag
        lookup k [(k', (k', lo', hi')) | (k', _, lo', hi') <- params]
      (x, more) <- case (inline, rest) of
        (Just x, _) -> Right (x, rest)
        (Nothing, x : xs) -> Right (x, xs)
        _ -> Left (flag <> ": expected one argument")
      let bad = name <> "=" <> x <> ": want an int in [" <> show lo <> ", " <> show hi <> "]"
          digits = if not (null x) && all isDigit x then Just (read x) else Nothing
      v <- maybe (Left bad) Right (mfilter (\y -> lo <= y && y <= hi) digits)
      go ((name, v) : vs) more

-- | Send n messages. `tick t snapshot` runs at t = 0, every interval and once more with the total, which
-- must equal the pure fold (Test.hs: fold_equivalence). An exception in any thread cancels all of them.
run :: Config -> (Double -> Summary -> IO ()) -> IO Summary
run cfg tick = do
  q <- newChan -- MVars: a write wakes one waiting sender, where an STM TBQueue write wakes all M
  room <- newQSem (capacity cfg) -- the producer waits while C are queued
  slots <- replicateM (senders cfg) (newIORef mempty) -- one single-writer Summary per sender
  t0 <- getMonotonicTime
  let p = fate cfg
      snapshot = mconcat <$> traverse readIORef slots -- each slot is a real Summary, so their sum is one
      report = do t <- getMonotonicTime; s <- snapshot; s <$ tick (t - t0) s
      produce = do
        forM_ (takeWhile (< n cfg) [0 ..]) $ \i -> do -- not [0 .. n - 1]: a Word64 wraps at n = 0
          msg <- evaluate (force (body alphabet (seed p) i)) -- here: laziness would move it to a sender
          waitQSem room >> writeChan q (Just (i, msg))
        writeChan q Nothing -- close: after the queued messages, one sentinel that every sender sees
      send slot = readChan q >>= \case
        Nothing -> writeChan q Nothing -- put it back for the next sender, then stop
        Just (i, _msg) -> do
          signalQSem room
          let (waits, s) = deliver p i -- the whole fate up front: retries stay with this sender
          mapM_ sleepUs waits
          modifyIORef' slot (<> s)
          send slot
  snapshot >>= tick 0
  let monitor = forever (threadDelay (intervalMs cfg * 1000) >> report)
  race_ monitor (concurrently_ produce (mapConcurrently_ send slots)) -- structured: any failure cancels all
  report

-- | The RTS rounds a sleep up to the next whole millisecond, so a zero wait would cost up to 1 ms: skip it.
sleepUs :: Integer -> IO ()
sleepUs us = when (us > 0) (threadDelay (fromInteger us))

-- | runRange in k sparks, one contiguous slice of [a, b) each; +RTS -N puts them on cores. The monoid makes
-- any split exact: parRange p k a b == runRange p a b for every k (Test.hs).
parRange :: Params -> Integer -> Word64 -> Word64 -> Summary
parRange p k a b = mconcat (parMap rseq (\j -> runRange p (cut j) (cut (j + 1))) [0 .. max 1 k - 1])
  where cut j = a + fromInteger (toInteger (b - a) * j `div` max 1 k)

-- | The Python monitor's fields: elapsed s, the Summary, mean service time per message (ms), rate (msg/s).
line :: Double -> Summary -> String
line t (Summary s f a us) =
  printf "t=%.2f sent=%d failed=%d attempts=%d time_us=%d mean_ms=%.2f rate=%.2f"
    t s f a us mean tput
  where
    done = fromInteger (s + f) :: Double
    mean = if done > 0 then fromInteger us / done / 1e3 else 0 :: Double
    tput = if t > 0 then done / t else 0

-- | The canonical last stdout line of every runner, as Python's json.dumps(..., separators=(",", ":")).
json :: Summary -> String
json (Summary s f a us) = printf "{\"sent\":%d,\"failed\":%d,\"attempts\":%d,\"time_us\":%d}" s f a us

main :: IO ()
main = do
  args <- getArgs
  let flag k = "--" <> map (\c -> if c == '_' then '-' else c) k
      usage = unwords ["[" <> flag k <> " " <> show d <> "]" | (k, d, _, _) <- params]
  if any (`elem` ["-h", "--help"]) args
    then putStrLn ("usage: sms-hs " <> usage <> " [+RTS -N<cores>]")
    else case parse args of
      Left e -> hPutStrLn stderr ("sms-hs: error: " <> e) >> exitWith (ExitFailure 2)
      Right cfg -> do
        hSetBuffering stderr LineBuffering -- whole monitor lines, even when the run cancels a tick
        total <- run cfg (\t s -> hPutStrLn stderr (line t s))
        putStrLn (json total)
