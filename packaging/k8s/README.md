# Kubernetes manifests

Two ways to try this on a cluster (a local one — [kind](https://kind.sigs.k8s.io/)
or [minikube](https://minikube.sigs.k8s.io/) — is easiest since these
reference `imagePullPolicy: IfNotPresent` and a `:local` tag, i.e. images
built locally, not pulled from a registry):

```sh
docker build -t stem-mqtt-broker:local   -f packaging/Dockerfile .
docker build -t stem-mqtt-demo-web:local -f demo/web/Dockerfile .

# kind only: load the images into the cluster's node
kind load docker-image stem-mqtt-broker:local stem-mqtt-demo-web:local
```

## Option A — one quick Pod (`demo-pod.yaml`)

Broker + demo webpage as two containers in a single Pod, sharing one
network namespace (so the webpage container can reach the broker at
`localhost` with no Service/DNS setup):

```sh
kubectl apply -f packaging/k8s/demo-pod.yaml
kubectl port-forward pod/stem-mqtt-demo 8090:80 8083:8083
open http://localhost:8090   # the page's default WS URL already matches
```

## Option B — Deployments + Services (`broker-deployment.yaml`, `demo-web-deployment.yaml`)

More realistic: broker and demo-web each get their own Deployment +
`NodePort` Service.

```sh
kubectl apply -f packaging/k8s/broker-deployment.yaml
kubectl apply -f packaging/k8s/demo-web-deployment.yaml
```

Then open the demo-web NodePort (`30809` by default — adjust for your
cluster's node IP, e.g. `minikube service stem-mqtt-demo-web --url`), and in
the page's **WebSocket URL** field, point it at the broker's `mqtt-ws`
NodePort (`30808` by default) instead of the built-in `ws://localhost:8083`
default — the page's browser JS can't resolve the in-cluster
`stem-mqtt-broker` Service DNS name, since it isn't running inside the
cluster.

## Cleanup

```sh
kubectl delete -f packaging/k8s/demo-pod.yaml
kubectl delete -f packaging/k8s/broker-deployment.yaml
kubectl delete -f packaging/k8s/demo-web-deployment.yaml
```
