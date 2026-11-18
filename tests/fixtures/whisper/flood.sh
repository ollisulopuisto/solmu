#!/bin/sh
head -c 2097152 /dev/zero | tr '\000' x
