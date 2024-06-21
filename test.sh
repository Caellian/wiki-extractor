#!/usr/bin/sh
for file in src; do
    echo mv $(ls $file/sta*.rs) $file
done
