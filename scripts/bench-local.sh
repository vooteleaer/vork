#!/usr/bin/env bash
# Throughput of one machine's CPU: several vØrk nodes on 127.0.0.1, one topology at a time.
# usage: bench-local.sh <vork binary> <topology: direct|relay1|relay2|multi2> <rate Mbit/s> [port base] [seconds]
# Prints the median received rate and loss after the first 3 seconds.
set -u
V=$(readlink -f "$1"); T=$2; R=$3; P=${4:-51850}; S=${5:-10}
D=$(mktemp -d /tmp/vork-bench.XXXX); cd "$D"
k(){ awk '/wg key/{print $3}' "$1.info"; }; h(){ awk '/address/{print $2}' "$1.info"; }
for n in a e b c; do "$V" keygen --out $n.key > $n.info; done
run(){ timeout $((S + 3)) "$V" run "$@" & }
L=127.0.0.1
case $T in
 direct)
  run --key b.key --listen $L:$((P+3)) > b.log 2>&1; sleep 0.3
  run --key c.key --listen $L:$((P+4)) --peer $L:$((P+3))@$(k b) --no-announce --bench $(h b) --bench-rate $R > c.log 2>&1 ;;
 relay1)
  run --key a.key --listen $L:$((P+1)) > a.log 2>&1; sleep 0.3
  run --key b.key --listen $L:$((P+3)) --peer $L:$((P+1))@$(k a) > b.log 2>&1; sleep 0.3
  run --key c.key --listen $L:$((P+4)) --peer $L:$((P+1))@$(k a) --no-announce --paths 1 --bench $(h b) --bench-rate $R > c.log 2>&1 ;;
 relay2)
  run --key a.key --listen $L:$((P+1)) > a.log 2>&1; sleep 0.3
  run --key e.key --listen $L:$((P+2)) --peer $L:$((P+1))@$(k a) > e.log 2>&1; sleep 0.3
  run --key b.key --listen $L:$((P+3)) --peer $L:$((P+2))@$(k e) > b.log 2>&1; sleep 0.5
  run --key c.key --listen $L:$((P+4)) --peer $L:$((P+1))@$(k a) --no-announce --paths 1 --bench $(h b) --bench-rate $R > c.log 2>&1 ;;
 multi2)
  run --key a.key --listen $L:$((P+1)) > a.log 2>&1
  run --key e.key --listen $L:$((P+2)) > e.log 2>&1; sleep 0.3
  run --key b.key --listen $L:$((P+3)) --peer $L:$((P+1))@$(k a) --peer $L:$((P+2))@$(k e) > b.log 2>&1; sleep 0.3
  run --key c.key --listen $L:$((P+4)) --peer $L:$((P+1))@$(k a) --peer $L:$((P+2))@$(k e) --no-announce --paths 2 --bench $(h b) --bench-rate $R > c.log 2>&1 ;;
 *) echo "unknown topology $T"; exit 1 ;;
esac
wait
med(){ sort -n | awk '{a[NR]=$1} END{print (NR ? a[int((NR+1)/2)] : "-")}'; }
rx=$(grep "Mbit/s received" b.log | tail -n +4 | awk '{for(i=1;i<=NF;i++) if($i=="Mbit/s") print $(i-1)}' | med)
loss=$(grep "Mbit/s received" b.log | tail -n +4 | grep -o "loss [0-9.]*" | awk '{print $2}' | med)
printf "%-7s sent %5s Mbit/s  received %7s Mbit/s  loss %5s%%\n" "$T" "$R" "$rx" "$loss"
cd /; rm -rf "$D"
