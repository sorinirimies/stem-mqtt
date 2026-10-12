{-# LANGUAGE OverloadedStrings #-}
-- Runtime smoke test for the Haskell bindings: client + broker over real
-- sockets. Prints "SMOKE OK" on success.
--
-- The generated Haskell bindings expose callback interfaces only as opaque
-- handles (no way to implement MqttMessageListener / MqttAuthProvider /
-- MqttBrokerEventListener in Haskell), so this test uses the *pull-style* API:
-- `mqttClientEnableMessageQueue`/`mqttClientNextMessage` and
-- `mqttBrokerEnableEventQueue`/`mqttBrokerNextEvent`. Authentication uses the
-- pull-style auth queue: each CONNECT becomes an AuthRequest the app pulls
-- (`mqttBrokerNextAuthRequest`) and answers (`mqttBrokerResolveAuthRequest`).
module Main (main) where

import Control.Concurrent (forkIO)
import Control.Concurrent.MVar (newEmptyMVar, putMVar, takeMVar)
import Control.Monad (unless)
import qualified Data.ByteString.Char8 as B
import Data.Text (Text)
import Data.Word (Word16)
import System.Exit (exitFailure)
import System.IO (hPutStrLn, stderr)

import UniFFI.MqttBroker
import qualified UniFFI.MqttClient as C

must :: Bool -> String -> IO ()
must cond what = unless cond $ hPutStrLn stderr ("FAIL " ++ what) >> exitFailure

ok :: Show e => String -> Either e a -> IO a
ok what (Left e) = hPutStrLn stderr ("FAIL " ++ what ++ ": " ++ show e) >> exitFailure
ok _ (Right a) = pure a

options :: Word16 -> Text -> Maybe Text -> C.ConnectOptions
options port clientId user =
  C.ConnectOptions "127.0.0.1" port clientId C.MqttVersionV5 True 30 user Nothing Nothing 10 5 False 0 0 Nothing 0 Nothing Nothing

-- Start a connect (it waits for the broker's verdict), answer the pulled auth
-- request according to the password the app 'checks', then return the result.
connectAnswering :: MqttBroker -> C.MqttClient -> Bool -> IO (Either C.MqttError C.ConnectResult)
connectAnswering broker client allow = do
  result <- newEmptyMVar
  _ <- forkIO (C.mqttClientConnect client >>= putMVar result)
  Just (AuthRequest reqId _ user _) <- mqttBrokerNextAuthRequest broker 5000
  must (user == Just "user") "auth request carries the username"
  _ <- mqttBrokerResolveAuthRequest broker reqId allow
  takeMVar result

main :: IO ()
main = do
  broker <-
    newMqttBroker
      (MqttBrokerConfig "127.0.0.1" 0 Nothing False 0 QoSExactlyOnce 100 100 0 Nothing 0 0 0)
  mqttBrokerEnableEventQueue broker 16
  mqttBrokerEnableAuthQueue broker 8
  ok "broker start" =<< mqttBrokerStart broker
  running <- mqttBrokerIsRunning broker
  must running "broker running"
  Just port <- mqttBrokerBoundPort broker
  must (port /= 0) "bound port"

  sub <- C.newMqttClient (options port "hs-sub" (Just "user"))
  C.mqttClientEnableMessageQueue sub 16 -- pull, don't push
  C.ConnectResult _ connectReason <- ok "sub connect" =<< connectAnswering broker sub True
  must (connectReason == 0) "connect accepted"
  C.SubscribeResult subReason <- ok "subscribe" =<< C.mqttClientSubscribe sub "smoke/#" C.QoSAtLeastOnce
  must (subReason < 0x80) "subscribe granted"

  pub <- C.newMqttClient (options port "hs-pub" (Just "user"))
  _ <- ok "pub connect" =<< connectAnswering broker pub True
  n <- mqttBrokerClientCount broker
  must (n == 2) "broker counts both clients"
  -- QoS 1 completes only after the broker's PUBACK.
  ok "publish" =<< C.mqttClientPublish pub "smoke/haskell" (B.pack "hello-haskell") C.QoSAtLeastOnce False

  -- The message arrives through the queue: no callback involved.
  delivered <- C.mqttClientNextMessage sub 5000
  case delivered of
    Just (C.MqttMessage topic payload _ _) -> do
      must (topic == "smoke/haskell") "message topic"
      must (payload == B.pack "hello-haskell") "message payload"
    Nothing -> must False "message delivered"

  bad <- C.newMqttClient (options port "hs-bad" (Just "user"))
  refused <- connectAnswering broker bad False
  must (either (const True) (const False) refused) "broker refuses when the app answers deny"

  -- The broker's events are pulled the same way.
  let drain acc = mqttBrokerNextEvent broker 300 >>= maybe (pure acc) (\e -> drain (e : acc))
  events <- drain []
  must (BrokerEventClientConnected "hs-sub" `elem` events) "event queue saw hs-sub"
  must (BrokerEventClientConnected "hs-pub" `elem` events) "event queue saw hs-pub"

  ok "pub disconnect" =<< C.mqttClientDisconnect pub
  ok "sub disconnect" =<< C.mqttClientDisconnect sub
  ok "broker stop" =<< mqttBrokerStop broker
  stopped <- mqttBrokerIsRunning broker
  must (not stopped) "broker stopped"
  putStrLn "SMOKE OK"
