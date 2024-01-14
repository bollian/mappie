#!/usr/bin/env python3

import mappie
import traceback


if __name__ == '__main__':
    try:
        mappie.main()
    except:
        print(traceback.format_exc())
